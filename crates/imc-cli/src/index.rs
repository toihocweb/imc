use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;

use imc_core::config::Config;
use imc_core::ingest::{build_manifest, language_of, read_text, FileEntry};
use imc_core::store;
use imc_parser;
use imc_search;

#[derive(Debug, Default, serde::Serialize)]
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

pub fn run(cfg: &Config, root: &Path, extra_ignores: &[String], force: bool) -> Result<IndexStats> {
    let start = std::time::Instant::now();
    let db = cfg.project_db(root);
    let mut store = store::open(&db)?;
    imc_search::rebuild_fts(&store)?;

    let manifest = build_manifest(root, cfg.max_file_bytes, extra_ignores)
        .context("walk project")?;

    let known = store.all_file_hashes();
    let mut new_files = Vec::new();
    let mut modified = Vec::new();
    let mut unchanged = 0usize;
    for f in &manifest {
        match known.iter().find(|(p, _)| p == &f.rel) {
            None => new_files.push(f),
            Some((_, h)) if force || h != &f.hash => modified.push(f),
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
    }
    store
        .conn
        .execute(
            "DELETE FROM files WHERE path NOT IN (SELECT path FROM files)",
            [],
        )
        .ok();
    for f in &manifest {
        if known.iter().all(|(p, _)| p != &f.rel) {
            continue;
        }
        store.upsert_file(&f.rel, &f.hash, language_of(&f.rel), f.size)?;
    }

    let todo: Vec<&FileEntry> = new_files.iter().chain(modified.iter()).copied().collect();
    let force_reindex = force;

    // Parse in parallel (tree-sitter is CPU bound); writes stay single-threaded.
    let parsed: Vec<_> = todo
        .par_iter()
        .map(|f| -> Result<_> {
            let text = read_text(&f.path)?;
            let parsed = imc_parser::parse(&f.rel, &text);
            let window_lines = 120;
            let mut chunks = Vec::new();
            let mut symbols = Vec::new();
            let mut edges = Vec::new();
            if let Some(p) = parsed {
                symbols = p.symbols;
                edges = p.edges;
                chunks = p.chunks;
            }
            if chunks.is_empty() {
                chunks = window_chunks(&f.rel, &text, window_lines);
            }
            Ok((f.rel.clone(), text.len(), symbols, edges, chunks))
        })
        .collect::<Result<Vec<_>>>()?;

    let embedder = imc_embed::build(&cfg.embedding);

    for (rel, size, symbols, edges, chunks) in parsed {
        store.clear_file(&rel)?;
        store.insert_symbols(&rel, &symbols)?;
        store.insert_edges(&rel, &edges)?;
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = embedder.embed(&texts)?;
        for (i, c) in chunks.iter().enumerate() {
            let cid = store.insert_chunk(&rel, c.start_line, c.end_line, &c.text, c.sym_id.as_deref())?;
            if let Some(v) = vectors.get(i) {
                store.insert_embedding(cid, v.len(), v)?;
            }
        }
        if force_reindex {
            // hashes already upserted above
        }
        let _ = size;
    }

    // Hashes for brand-new files that never made it into the pre-pass.
    for f in &manifest {
        if known.iter().all(|(p, _)| p != &f.rel) {
            store.upsert_file(&f.rel, &f.hash, language_of(&f.rel), f.size)?;
        }
    }
    imc_search::rebuild_fts(&store)?;

    let stats = IndexStats {
        new_files: new_files.len(),
        modified_files: modified.len(),
        deleted_files: deleted.len(),
        unchanged_files: unchanged,
        total_files: manifest.len(),
        symbols: store.count_symbols(),
        edges: store.count_edges(),
        chunks: store.count_chunks(),
        embedder: embedder.name(),
        elapsed_ms: start.elapsed().as_millis(),
    };
    Ok(stats)
}

/// Sliding-window chunking for files without AST symbols (markdown, configs, plain text).
pub fn window_chunks(rel: &str, text: &str, window: usize) -> Vec<imc_parser::RawChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let header = format!("{} (lines 1-{})", rel, lines.len());
    let mut out = Vec::new();
    let step = window / 2;
    let mut i = 0;
    while i < lines.len() {
        let end = (i + window).min(lines.len());
        let body = lines[i..end].join("\n");
        out.push(imc_parser::RawChunk {
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

pub fn report(s: &IndexStats) {
    println!("Indexed with {} in {} ms", s.embedder, s.elapsed_ms);
    println!(
        "  files: {} (new {}, modified {}, deleted {}, unchanged {})",
        s.total_files, s.new_files, s.modified_files, s.deleted_files, s.unchanged_files
    );
    println!("  symbols: {}", s.symbols);
    println!("  edges:   {}", s.edges);
    println!("  chunks:  {}", s.chunks);
}