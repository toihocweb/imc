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

pub fn run(cfg: &Config, root: &Path, extra_ignores: &[String], force: bool) -> Result<IndexStats> {
    let start = std::time::Instant::now();
    let db = cfg.project_db(root);
    let mut store = store::open(&db)?;
    imc_search::rebuild_fts(&store)?;

    let manifest = build_manifest(root, cfg.max_file_bytes, extra_ignores)
        .context("walk project")?;

    let known = store.all_file_hashes();
    let mut new_files: Vec<&FileEntry> = Vec::new();
    let mut modified: Vec<&FileEntry> = Vec::new();
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
        store.remove_file(rel)?;
    }
    for f in &manifest {
        store.upsert_file(&f.rel, &f.hash, language_of(&f.rel), f.size)?;
    }

    let todo: Vec<&FileEntry> = new_files.iter().chain(modified.iter()).copied().collect();

    // Parse in parallel (tree-sitter is CPU bound); writes stay single-threaded.
    let window_lines = cfg.window_lines;
    let parsed: Vec<_> = todo
        .par_iter()
        .map(|f| -> Result<(String, Vec<imc_core::types::Symbol>, Vec<imc_core::types::Edge>, Vec<RawChunk>)> {
            let text = read_text(&f.path)?;
            let (mut symbols, mut edges, mut chunks) = match imc_parser::parse(&f.rel, &text) {
                Some(p) => (p.symbols, p.edges, p.chunks),
                None => (Vec::new(), Vec::new(), Vec::new()),
            };
            if chunks.is_empty() {
                chunks = window_chunks(&f.rel, &text, window_lines);
            }
            Ok((f.rel.clone(), symbols, edges, chunks))
        })
        .collect::<Result<Vec<_>>>()?;

    let embedder = imc_embed::build(&cfg.embedding);
    let embedder_name = embedder.as_ref().map(|e| e.name()).unwrap_or_else(|| "none".into());

    for (rel, symbols, edges, chunks) in parsed {
        store.clear_file(&rel)?;
        store.insert_symbols(&rel, &symbols)?;
        store.insert_edges(&rel, &edges)?;
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = match &embedder {
            Some(e) => e.embed(&texts)?,
            None => Vec::new(),
        };
        for (i, c) in chunks.iter().enumerate() {
            // Path + line range inside the indexed text: filename queries
            // (`parser.py`, `mcp/server.py`) must hit FTS directly.
            let text = format!("{}:{}-{}\n{}", rel, c.start_line, c.end_line, c.text);
            let cid = store.insert_chunk(&rel, c.start_line, c.end_line, &text, c.sym_id.as_deref())?;
            // A failed input comes back as an empty vector — skip rather than
            // persist a 0-dim row that cosine can never score.
            if let Some(v) = vectors.get(i).filter(|v| !v.is_empty()) {
                store.insert_embedding(cid, v.len(), v)?;
            }
        }
    }

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

/// Sliding-window chunking for files without AST symbols (markdown, configs, plain text).
pub fn window_chunks(rel: &str, text: &str, window: usize) -> Vec<RawChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let header = format!("{} (lines 1-{})", rel, lines.len());
    let mut out = Vec::new();
    let step = (window / 2).max(1);
    let mut i = 0;
    while i < lines.len() {
        let end = (i + window).min(lines.len());
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