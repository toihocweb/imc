//! Indexing pipeline: walk → incremental diff → parallel tree-sitter parse →
//! chunk → embed → store → refresh FTS. Extracted from the CLI so the HTTP API
//! and the MCP server share exactly one implementation.

use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;

use imc_core::config::Config;
use imc_core::ingest::{build_manifest, language_of, read_text, FileEntry};
use imc_core::store;
use imc_parser::RawChunk;
use imc_search;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexStats {
    pub new_files: usize,
    pub modified_files: usize,
    pub deleted_files: usize,
    pub unchanged_files: usize,
    pub total_files: usize,
    pub symbols: usize,
    pub edges: usize,
    pub chunks: usize,
    pub embedder: String,
    pub elapsed_ms: u128,
}

/// The stage an index run is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Walking the tree.
    Scan,
    /// Parsing sources with tree-sitter.
    Parse,
    /// Embedding and writing chunks.
    Embed,
    /// Rebuilding the search index.
    Finalize,
}

/// A progress tick. Emitted while indexing so a long run is not a black box.
#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    /// Files done in this phase.
    pub done: usize,
    /// Files in this phase (0 when unknown).
    pub total: usize,
    /// The file being worked on.
    pub current: String,
}

pub fn run(cfg: &Config, root: &Path, extra_ignores: &[String], force: bool) -> Result<IndexStats> {
    run_with_progress(cfg, root, extra_ignores, force, &|_| {})
}

/// Index a project, reporting progress as it goes.
///
/// The callback is called from the rayon workers during the parse phase, so it
/// has to be `Sync`; the caller decides how much to print.
pub fn run_with_progress(
    cfg: &Config,
    root: &Path,
    extra_ignores: &[String],
    force: bool,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<IndexStats> {
    let start = std::time::Instant::now();
    let db = cfg.project_db(root);
    let mut store = store::open(&db)?;
    imc_search::rebuild_fts(&store)?;

    progress(Progress {
        phase: Phase::Scan,
        done: 0,
        total: 0,
        current: String::new(),
    });
    let manifest = build_manifest(root, cfg.max_file_bytes, extra_ignores)
        .context("walk project")?;
    progress(Progress {
        phase: Phase::Scan,
        done: manifest.len(),
        total: manifest.len(),
        current: String::new(),
    });

    // A different chunker, strategy, budget or embedding cap means the stored
    // chunks are stale even though the files did not change.
    let chunk_signature = format!(
        "{CHUNKER_VERSION}|{}|{}|{}",
        cfg.chunk_strategy, cfg.chunk_budget_nws, EMBED_TEXT_CAP
    );
    let stored_signature = store.get_meta(CHUNK_SIGNATURE_KEY)?.unwrap_or_default();
    let chunker_changed = !force && stored_signature != chunk_signature;
    if chunker_changed {
        eprintln!(
            "[imc] chunking changed ({}) — re-indexing the whole project",
            if stored_signature.is_empty() {
                "no version recorded".to_string()
            } else {
                stored_signature
            }
        );
    }

    let known = store.all_file_hashes();
    let mut new_files: Vec<&FileEntry> = Vec::new();
    let mut modified: Vec<&FileEntry> = Vec::new();
    let mut unchanged = 0usize;
    for f in &manifest {
        match known.iter().find(|(p, _)| p == &f.rel) {
            None => new_files.push(f),
            Some((_, h)) if force || chunker_changed || h != &f.hash => modified.push(f),
            _ => unchanged += 1,
        }
    }
    let new_paths: std::collections::HashSet<&String> = manifest.iter().map(|f| &f.rel).collect();
    let deleted: Vec<String> = known
        .iter()
        .map(|(p, _)| p.clone())
        .filter(|p| !new_paths.contains(p))
        .collect();
    for rel in &deleted {
        store.clear_file(rel)?;
        store.remove_file(rel)?;
    }
    for f in &manifest {
        store.upsert_file(&f.rel, &f.hash, language_of(&f.rel), f.size)?;
    }

    let todo: Vec<&FileEntry> = new_files.iter().chain(modified.iter()).copied().collect();

    // Parse in parallel (tree-sitter is CPU bound); writes stay single-threaded.
    let window_lines = cfg.window_lines;
    let budget_nws = cfg.chunk_budget_nws;
    let strategy = cfg.chunk_strategy.clone();
    let todo_total = todo.len();
    let parsed_counter = std::sync::atomic::AtomicUsize::new(0);
    let parsed: Vec<_> = todo
        .par_iter()
        .map(|f| -> Result<(String, Vec<imc_core::types::Symbol>, Vec<imc_core::types::Edge>, Vec<RawChunk>)> {
            let text = read_text(&f.path)?;
            let done = parsed_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            progress(Progress {
                phase: Phase::Parse,
                done,
                total: todo_total,
                current: f.rel.clone(),
            });
            let (symbols, edges, mut chunks) =
                match imc_parser::parse_with(&f.rel, &text, budget_nws, &strategy) {
                    Some(p) => (p.symbols, p.edges, p.chunks),
                    None => (Vec::new(), Vec::new(), Vec::new()),
                };
            if chunks.is_empty() {
                chunks = window_chunks(&f.rel, &text, window_lines, budget_nws);
            }
            Ok((f.rel.clone(), symbols, edges, chunks))
        })
        .collect::<Result<Vec<_>>>()?;

    let embedder = imc_embed::build(&cfg.embedding);
    let embedder_name = embedder.as_ref().map(|e| e.name()).unwrap_or_else(|| "none".into());

    // The embedding is one pass over every chunk, not one request per file.
    //
    // The indexer used to call `embed()` inside the per-file loop, so a
    // request carried one file's chunks and the requests ran one after the
    // other: indexing 40 files took 67 seconds, almost all of it waiting on
    // the endpoint. Collecting the texts first lets the embedder fill its
    // batches and run them concurrently; the database writes stay in file
    // order, single-threaded, as before.
    let parsed_total = parsed.len();
    let mut embed_texts: Vec<String> = Vec::new();
    // (chunk id, index into embed_texts)
    let mut pending: Vec<(i64, usize)> = Vec::new();

    for (done, (rel, symbols, edges, chunks)) in parsed.into_iter().enumerate() {
        progress(Progress {
            phase: Phase::Embed,
            done: done + 1,
            total: parsed_total,
            current: rel.clone(),
        });
        store.clear_file(&rel)?;
        store.insert_symbols(&rel, &symbols)?;
        store.insert_edges(&rel, &edges)?;
        for c in chunks.iter() {
            // Path + line range inside the indexed text: filename queries
            // (`parser.py`, `mcp/server.py`) must hit FTS directly.
            let text = format!("{}:{}-{}\n{}", rel, c.start_line, c.end_line, c.text);
            let cid = store.insert_chunk(&rel, c.start_line, c.end_line, &text, c.sym_id.as_deref())?;
            if embedder.is_some() {
                // The embedding sees at most ~4 KB of the chunk: the vector leg
                // only needs the head of the code (signature + docstring), and
                // the API rejects a huge input. The stored text stays whole,
                // so the lexical leg still matches the tail of a long function.
                embed_texts.push(cap_for_embedding(&c.text, EMBED_TEXT_CAP));
                pending.push((cid, embed_texts.len() - 1));
            }
        }
    }

    if !pending.is_empty() {
        progress(Progress {
            phase: Phase::Embed,
            done: 0,
            total: pending.len(),
            current: format!("{} chunks", pending.len()),
        });
        let vectors = match &embedder {
            Some(e) => e.embed(&embed_texts)?,
            None => Vec::new(),
        };
        for (cid, idx) in pending {
            // A failed input comes back as an empty vector — skip rather than
            // persist a 0-dim row that cosine can never score.
            if let Some(v) = vectors.get(idx).filter(|v| !v.is_empty()) {
                store.insert_embedding(cid, v.len(), v)?;
            }
        }
    }

    store.set_meta(CHUNK_SIGNATURE_KEY, &chunk_signature)?;

    progress(Progress {
        phase: Phase::Finalize,
        done: 0,
        total: 0,
        current: String::new(),
    });
    imc_search::rebuild_fts(&store)?;

    Ok(IndexStats {
        new_files: new_files.len(),
        modified_files: modified.len(),
        deleted_files: deleted.len(),
        unchanged_files: unchanged,
        total_files: manifest.len(),
        symbols: store.count_symbols(),
        edges: store.count_edges(),
        chunks: store.count_chunks(),
        embedder: embedder_name,
        elapsed_ms: start.elapsed().as_millis(),
    })
}

/// Bumped whenever the chunk text changes shape.
///
/// The chunker's output is what the index stores, and an incremental run only
/// re-embeds the files whose content changed. A chunker change therefore
/// leaves the old chunks in place: the index ends up half old, half new, and
/// the vectors no longer match the text. The version is written into the
/// project database and compared on the next run, so a chunker change
/// re-indexes the project by itself.
pub const CHUNKER_VERSION: &str = "2";

/// The meta key the chunk signature is stored under.
pub const CHUNK_SIGNATURE_KEY: &str = "chunk_signature";

/// How much of a chunk is sent to the embedding model.
const EMBED_TEXT_CAP: usize = 4096;

/// Cap a text for the embedding model, on a character boundary.
fn cap_for_embedding(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Non-whitespace characters — the size unit the cAST paper budgets in.
fn nws_len(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// Sliding-window chunking for files without AST symbols (markdown, configs, plain text).
///
/// The window is capped at the same non-whitespace budget the AST chunking
/// uses (cAST paper: ~2,000 non-whitespace characters).
pub fn window_chunks(
    rel: &str,
    text: &str,
    window: usize,
    budget_nws: usize,
) -> Vec<RawChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let header = format!("{} (lines 1-{})", rel, lines.len());
    let mut out = Vec::new();
    let step = (window / 2).max(1);
    let mut i = 0;
    while i < lines.len() {
        let mut end = (i + window).min(lines.len());
        // Shrink the window until it fits the budget: an uncapped 120-line
        // window produced a single 8.4 KB chunk, four times the embedding
        // budget of a normal chunk for no extra signal.
        while end > i + 1 && nws_len(&lines[i..end].join("\n")) > budget_nws {
            end -= 1;
        }
        let body = lines[i..end].join("\n");
        out.push(RawChunk {
            start_line: i + 1,
            end_line: end,
            text: format!("{header}\n{body}"),
            sym_id: None,
        });
        if end == lines.len() {
            break;
        }
        i += step;
    }
    out
}