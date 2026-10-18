//! Project memory: durable facts learned about a codebase, stored in SQLite
//! with the same vector+lexical retrieval shape as the Python
//! `main/memory/{store,retrieval,extractor}.py` modules.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use imc_core::store::Store;
use imc_embed::tokenize;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub category: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub importance: f64,
    pub use_count: i64,
    pub created_at: i64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryHit {
    pub memory: Memory,
    pub score: f64,
    pub matched_paths: Vec<String>,
}

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    content_norm TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT 'note',
    paths TEXT NOT NULL DEFAULT '[]',
    tags TEXT NOT NULL DEFAULT '[]',
    importance REAL NOT NULL DEFAULT 0.5,
    use_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual'
);
CREATE INDEX IF NOT EXISTS idx_memories_cat ON memories(category);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    content,
    content='memories',
    content_rowid='id',
    tokenize='unicode61'
);
CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;
CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content);
END;
"#;

pub fn ensure_schema(store: &Store) -> Result<()> {
    store.conn.execute_batch(DDL).context("memory schema")?;
    Ok(())
}

pub fn normalize(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Insert a memory; exact-duplicate content is rejected like the Python store.
pub fn add(
    store: &Store,
    content: &str,
    category: &str,
    paths: &[String],
    tags: &[String],
    importance: f64,
    source: &str,
) -> Result<Option<i64>> {
    ensure_schema(store)?;
    let norm = normalize(content);
    if norm.is_empty() {
        return Ok(None);
    }
    let dup: Option<i64> = store
        .conn
        .query_row(
            "SELECT id FROM memories WHERE content_norm = ?1",
            params![norm],
            |r| r.get(0),
        )
        .optional()?;
    if dup.is_some() {
        return Ok(None);
    }
    store.conn.execute(
        "INSERT INTO memories(content, content_norm, category, paths, tags, importance, created_at, source)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            content,
            norm,
            category,
            serde_json::to_string(paths)?,
            serde_json::to_string(tags)?,
            importance.clamp(0.0, 1.0),
            now(),
            source
        ],
    )?;
    Ok(Some(store.conn.last_insert_rowid()))
}

pub fn get(store: &Store, id: i64) -> Result<Option<Memory>> {
    ensure_schema(store)?;
    Ok(store
        .conn
        .query_row(
            "SELECT id, content, category, paths, tags, importance, use_count, created_at, source
             FROM memories WHERE id=?1",
            params![id],
            row_to_memory,
        )
        .optional()?)
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let paths: String = r.get(3)?;
    let tags: String = r.get(4)?;
    Ok(Memory {
        id: r.get(0)?,
        content: r.get(1)?,
        category: r.get(2)?,
        paths: serde_json::from_str(&paths).unwrap_or_default(),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        importance: r.get(5)?,
        use_count: r.get(6)?,
        created_at: r.get(7)?,
        source: r.get(8)?,
    })
}

pub fn list(store: &Store, limit: usize) -> Result<Vec<Memory>> {
    ensure_schema(store)?;
    let mut stmt = store.conn.prepare(
        "SELECT id, content, category, paths, tags, importance, use_count, created_at, source
         FROM memories ORDER BY importance DESC, created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    Ok(rows.flatten().collect())
}

pub fn delete(store: &Store, id: i64) -> Result<bool> {
    ensure_schema(store)?;
    let n = store
        .conn
        .execute("DELETE FROM memories WHERE id=?1", params![id])?;
    Ok(n > 0)
}

pub fn touch(store: &Store, id: i64) -> Result<()> {
    ensure_schema(store)?;
    store.conn.execute(
        "UPDATE memories SET use_count = use_count + 1 WHERE id=?1",
        params![id],
    )?;
    Ok(())
}

/// Merge duplicate rows whose normalised content matches (the Python
/// `deduplicate_docstore` pass).
pub fn deduplicate(store: &Store) -> Result<usize> {
    ensure_schema(store)?;
    let mut stmt = store.conn.prepare(
        "SELECT content_norm, MIN(id), COUNT(*) FROM memories GROUP BY content_norm HAVING COUNT(*) > 1",
    )?;
    let dupes: Vec<(String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .flatten()
        .collect();
    let mut removed = 0usize;
    for (_, keep, count) in dupes {
        removed += (count - 1) as usize;
        store.conn.execute(
            "DELETE FROM memories WHERE content_norm = (SELECT content_norm FROM memories WHERE id=?1) AND id > ?1",
            params![keep],
        )?;
    }
    Ok(removed)
}

fn lexical_scores(store: &Store, query: &str, limit: usize) -> HashMap<i64, f64> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return HashMap::new();
    }
    let expr = tokens
        .iter()
        .map(|t| format!("\"{}\" OR \"{}\"*", t.replace('"', ""), t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT rowid, bm25(memories_fts) FROM memories_fts WHERE memories_fts MATCH ?1 LIMIT ?2"
    );
    let mut stmt = match store.conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };
    let rows: Vec<(i64, f64)> = stmt
        .query_map(params![expr, limit as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default();
    rows.into_iter().map(|(id, bm)| (id, -bm)).collect()
}

/// Ranked memory retrieval: lexical + optional vector + path boost + importance
/// and usage decay, mirroring the Python retrieval weighting.
pub fn retrieve(
    store: &Store,
    embedder: Option<&dyn imc_embed::Embedder>,
    query: &str,
    active_paths: &[String],
    limit: usize,
) -> Result<Vec<MemoryHit>> {
    ensure_schema(store)?;
    let all = list(store, limit * 6)?;
    if all.is_empty() {
        return Ok(Vec::new());
    }
    let lex = lexical_scores(store, query, limit * 6);

    let query_vec = embedder
        .and_then(|e| e.embed(&[query.to_string()]).ok())
        .and_then(|v| v.into_iter().next());
    let mut vectors: HashMap<i64, Vec<f32>> = HashMap::new();
    if let Some(qv) = &query_vec {
        let mut stmt = match store
            .conn
            .prepare("SELECT mem_id, vec FROM memory_vectors WHERE mem_id IN (SELECT id FROM memories)")
        {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()),
        };
        let rows = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                let mut v = Vec::with_capacity(blob.len() / 4);
                for c in blob.chunks_exact(4) {
                    v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
                Ok((id, v))
            })
            .map(|it| it.filter_map(Result::ok).collect::<Vec<_>>())
            .unwrap_or_default();
        for (id, v) in rows {
            if v.len() == qv.len() {
                vectors.insert(id, v);
            }
        }
    }

    let path_set: HashSet<&String> = active_paths.iter().collect();
    let mut hits = Vec::new();
    for m in all {
        let mut score = m.importance * 0.25;
        let mut matched_paths = Vec::new();
        if let Some(l) = lex.get(&m.id) {
            score += l * 0.9;
        }
        if let Some((_, v)) = vectors.get(&m.id).zip(query_vec.as_ref()) {
            score += cosine(v, query_vec.as_ref().unwrap()) * 0.6;
        }
        for p in &m.paths {
            if path_set.contains(p) {
                score += 0.5;
                matched_paths.push(p.clone());
            }
        }
        for t in tokenize(query) {
            if m.tags.iter().any(|mt| mt.eq_ignore_ascii_case(&t)) {
                score += 0.2;
            }
        }
        score += (m.use_count as f64).min(10.0) * 0.01;
        if score > 0.15 {
            hits.push(MemoryHit {
                memory: m,
                score,
                matched_paths,
            });
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    Ok(hits)
}

pub fn embed_and_store(
    store: &Store,
    embedder: Option<&dyn imc_embed::Embedder>,
    id: i64,
    content: &str,
) -> Result<()> {
    let Some(v) = embedder
        .and_then(|e| e.embed(&[content.to_string()]).ok())
        .and_then(|v| v.into_iter().next()) else {
        return Ok(());
    };
    let mut buf = Vec::with_capacity(v.len() * 4);
    for f in &v {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    store.conn.execute(
        "CREATE TABLE IF NOT EXISTS memory_vectors (mem_id INTEGER PRIMARY KEY, dim INTEGER, vec BLOB)",
        [],
    )?;
    store.conn.execute(
        "INSERT OR REPLACE INTO memory_vectors(mem_id, dim, vec) VALUES (?1,?2,?3)",
        params![id, v.len() as i64, buf],
    )?;
    Ok(())
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..a.len() {
        dot += a[i] as f64 * b[i] as f64;
        na += a[i] as f64 * a[i] as f64;
        nb += b[i] as f64 * b[i] as f64;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Rule-based extraction of candidate memories from a transcript/session text.
/// LLM-based extraction stays optional; this keeps `imc learn` dependency-free.
pub fn extract_candidates(text: &str, active_paths: &[String]) -> Vec<MemoryCandidate> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut push = |content: String, category: &str, source: &str| {
        let norm = normalize(&content);
        if norm.len() < 20 || !seen.insert(norm) {
            return;
        }
        let paths = active_paths
            .iter()
            .filter(|p| content.contains(p.as_str()))
            .cloned()
            .collect();
        out.push(MemoryCandidate {
            content,
            category: category.to_string(),
            paths,
            tags: Vec::new(),
            source: source.to_string(),
        });
    };

    // Convention lines: "Rule: ...", "Note: ...", "Convention: ...", "TODO: ..."
    for line in text.lines() {
        let t = line.trim();
        for prefix in ["RULE:", "Rule:", "NOTE:", "Note:", "CONVENTION:", "Convention:", "ALWAYS:", "Always:"] {
            if let Some(rest) = t.strip_prefix(prefix) {
                if rest.len() > 20 {
                    push(rest.trim().to_string(), "rule", "auto");
                }
            }
        }
    }

    // Decisions recorded as "we decided to X" / "chose X over Y".
    for line in text.lines() {
        let t = line.trim();
        let lower = t.to_lowercase();
        if (lower.contains("we decided") || lower.contains("chose ") || lower.contains("going with "))
            && t.len() > 25
        {
            push(t.to_string(), "decision", "auto");
        }
    }
    out.truncate(25);
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryCandidate {
    pub content: String,
    pub category: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub source: String,
}