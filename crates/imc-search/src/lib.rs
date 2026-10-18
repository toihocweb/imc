//! Hybrid search: SQLite FTS5 (BM25) + vector cosine + reciprocal-rank fusion,
//! annotated with bounded AST graph evidence. Replaces
//! `main/search/{semantic,graph,context}.py`.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::params;

use imc_core::store::Store;
use imc_core::types::{SearchHit, Symbol};
use imc_embed::tokenize;

const RRF_K: f64 = 60.0;
const CANDIDATES: usize = 60;

/// Create the FTS5 index + triggers if missing (external content over `chunks`).
pub fn ensure_fts(store: &Store) -> Result<()> {
    store.conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            text,
            content='chunks',
            content_rowid='id',
            tokenize='unicode61'
        );
        CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
            INSERT INTO chunks_fts(rowid, text) VALUES (new.id, new.text);
        END;
        CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
            INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES ('delete', old.id, old.text);
        END;
        "#,
    )?;
    Ok(())
}

/// Rebuild FTS from scratch — cheap for a full index run and always consistent.
pub fn rebuild_fts(store: &Store) -> Result<()> {
    ensure_fts(store)?;
    store
        .conn
        .execute_batch(
            "DELETE FROM chunks_fts;
             INSERT INTO chunks_fts(rowid, text) SELECT id, text FROM chunks;",
        )
        .context_debug("rebuild fts")?;
    Ok(())
}

trait ContextDebug<T> {
    fn context_debug(self, msg: &str) -> Result<T>;
}

impl<T> ContextDebug<T> for Result<T, rusqlite::Error> {
    fn context_debug(self, msg: &str) -> Result<T> {
        self.map_err(|e| anyhow::anyhow!("{msg}: {e}"))
    }
}

fn fts_query(raw: &str) -> String {
    // Escape FTS5 special chars: quote each token, prefix-match the last one.
    let tokens = tokenize(raw);
    if tokens.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = tokens.iter().map(|t| format!("\"{t}\"")).collect();
    if tokens.last().is_some() {
        let last = parts.pop().unwrap();
        parts.push(format!("\"{}\"*", last.trim_matches('"')));
    }
    parts.join(" ")
}

fn bm25_scores(store: &Store, query: &str) -> Result<HashMap<i64, f64>> {
    let q = fts_query(query);
    if q.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = store.conn.prepare(
        "SELECT rowid, bm25(chunks_fts) FROM chunks_fts WHERE chunks_fts MATCH ?1 LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![q, CANDIDATES as i64], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
    })?;
    let mut out = HashMap::new();
    // bm25() is negative (lower = better); invert into a positive score.
    for row in rows.flatten() {
        out.insert(row.0, -row.1);
    }
    Ok(out)
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        let x = a[i] as f64;
        let y = b[i] as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn vector_scores(store: &Store, query_vec: &[f32]) -> HashMap<i64, f64> {
    let mut scored: Vec<(i64, f64)> = store
        .all_vectors()
        .into_iter()
        .filter(|(_, v)| v.len() == query_vec.len())
        .map(|(id, v)| (id, cosine(query_vec, &v)))
        .filter(|(_, s)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(CANDIDATES);
    scored.into_iter().collect()
}

fn rrf(rank_lists: &[HashMap<i64, f64>]) -> Vec<(i64, f64)> {
    let mut fused: HashMap<i64, f64> = HashMap::new();
    for list in rank_lists {
        // Convert score maps to rank positions.
        let mut items: Vec<(i64, f64)> = list.iter().map(|(k, v)| (*k, *v)).collect();
        items.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (rank, (id, _)) in items.into_iter().enumerate() {
            *fused.entry(id).or_insert(0.0) += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut out: Vec<(i64, f64)> = fused.into_iter().collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Lexical-ish boost: symbols whose name matches query tokens.
fn symbol_name_boost(store: &Store, query: &str) -> HashMap<String, f64> {
    let tokens = tokenize(query);
    let mut boost = HashMap::new();
    for t in tokens.iter().take(12) {
        for sym in store.find_symbols_by_name(t, 50) {
            *boost.entry(sym.file.clone()).or_insert(0.0) += 0.15;
            *boost.entry(sym.name.clone()).or_insert(0.0) += 0.30;
        }
    }
    boost
}

fn snippet(text: &str, query: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let lower = text.to_lowercase();
    let tokens = tokenize(query);
    let mut pos = None;
    for t in &tokens {
        if let Some(p) = lower.find(t.as_str()) {
            pos = Some(p);
            break;
        }
    }
    let start = match pos {
        Some(p) => text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i < p.saturating_sub(120))
            .last()
            .map(|i| i + 120.min(p))
            .unwrap_or(0),
        None => 0,
    };
    let start = start.min(text.len().saturating_sub(max_chars));
    let mut end = (start + max_chars).min(text.len());
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut slice = text[start..end].to_string();
    if start > 0 {
        slice = format!("…{slice}");
    }
    if end < text.len() {
        slice.push('…');
    }
    slice
}

/// Graph evidence: definitions in the hit file whose names appear in the chunk,
/// plus direct callees/callers of the hit symbol.
fn graph_evidence(
    store: &Store,
    hit_file: &str,
    hit_sym: Option<&str>,
    text: &str,
) -> Vec<String> {
    let mut evidence = Vec::new();
    let defs: Vec<Symbol> = store.definitions_in_file(hit_file);
    let mut seen = HashSet::new();

    for d in defs.iter() {
        if Some(d.id.as_str()) == hit_sym {
            continue;
        }
        if mentions(&d.name, text) && seen.insert(d.id.clone()) {
            evidence.push(format!("{}:{} {}", d.file, d.line, d.qualname));
            if evidence.len() >= 4 {
                break;
            }
        }
    }

    if let Some(sid) = hit_sym {
        for callee in store.callees(sid).into_iter().take(4) {
            // Resolve to a definition for a readable name.
            let resolved = store
                .find_symbols_by_name(&callee, 1)
                .into_iter()
                .next()
                .map(|s| format!("{} ({})", s.qualname, s.file))
                .unwrap_or(callee);
            if seen.insert(format!("c:{resolved}")) {
                evidence.push(format!("→ calls {resolved}"));
            }
        }
        if let Some(sym) = store.find_symbols_by_name(sid.rsplit("::").next().unwrap_or(""), 1).into_iter().next() {
            for caller in store.callers(&sym.name).into_iter().take(3) {
                let short = caller.split("::").last().unwrap_or(&caller).to_string();
                if seen.insert(format!("r:{caller}")) {
                    evidence.push(format!("← called by {short}"));
                }
            }
        }
    }
    evidence.truncate(6);
    evidence
}

fn mentions(name: &str, text: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let nb = name.as_bytes();
    let mut start = 0;
    while let Some(p) = text[start..].find(name) {
        let abs = start + p;
        let before_ok = abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let after = abs + nb.len();
        let after_ok =
            after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
        if start >= text.len() {
            break;
        }
    }
    false
}

pub struct SearchOptions {
    pub limit: usize,
    pub vector: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: 8,
            vector: true,
        }
    }
}

pub fn search(
    store: &Store,
    embedder: &dyn imc_embed::Embedder,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let mut lists: Vec<HashMap<i64, f64>> = Vec::new();

    let bm = bm25_scores(store, query)?;
    if !bm.is_empty() {
        lists.push(bm);
    }

    if opts.vector {
        if let Ok(qv) = embedder.embed(&[query.to_string()]) {
            if let Some(v) = qv.into_iter().next() {
                let vs = vector_scores(store, &v);
                if !vs.is_empty() {
                    lists.push(vs);
                }
            }
        }
    }

    if lists.is_empty() {
        return Ok(Vec::new());
    }

    let fused = rrf(&lists);
    let file_boost = symbol_name_boost(store, query);

    let mut hits = Vec::new();
    for (chunk_id, score) in fused.into_iter().take(opts.limit * 4) {
        let Some(chunk) = store.chunk_by_id(chunk_id) else {
            continue;
        };
        let mut s = score;
        if let Some(b) = file_boost.get(&chunk.file) {
            s += b * 0.01;
        }
        let (sym_name, sym_kind) = match &chunk.sym_id {
            Some(sid) => {
                let short = sid.rsplit("::").next().unwrap_or("").to_string();
                let kind = store
                    .find_symbols_by_name(&short, 8)
                    .into_iter()
                    .find(|s| &s.id == sid)
                    .map(|s| s.kind.as_str().to_string());
                (Some(short), kind)
            }
            None => (None, None),
        };
        let evidence = graph_evidence(store, &chunk.file, chunk.sym_id.as_deref(), &chunk.text);
        let mut text = chunk.text.clone();
        if !evidence.is_empty() {
            text.push_str(&format!("\n// graph: {}", evidence.join("; ")));
        }
        hits.push(SearchHit {
            score: s,
            file: chunk.file.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            snippet: snippet(&chunk.text, query, 600),
            text,
            sym_name,
            sym_kind,
            source: "hybrid",
        });
        if hits.len() >= opts.limit {
            break;
        }
    }
    Ok(hits)
}