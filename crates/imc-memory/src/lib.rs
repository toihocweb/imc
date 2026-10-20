//! Project memory: durable facts learned about a codebase, stored in SQLite
//! with the same vector+lexical retrieval shape as the Python
//! `main/memory/{store,retrieval,extractor}.py` modules.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use imc_core::store::Store;
use imc_embed::tokenize;

/// Python `MemoryType` — the five work-memory categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    WorkFact,
    WorkTask,
    WorkMethod,
    WorkArtifact,
    BusinessRule,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::WorkFact => "work_fact",
            MemoryType::WorkTask => "work_task",
            MemoryType::WorkMethod => "work_method",
            MemoryType::WorkArtifact => "work_artifact",
            MemoryType::BusinessRule => "business_rule",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "work_task" => MemoryType::WorkTask,
            "work_method" => MemoryType::WorkMethod,
            "work_artifact" => MemoryType::WorkArtifact,
            "business_rule" => MemoryType::BusinessRule,
            _ => MemoryType::WorkFact,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    /// Python `type` — one of the five work-memory kinds.
    pub mem_type: MemoryType,
    /// Python `scene_name` — the working scene this memory came from.
    pub scene: String,
    pub category: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub importance: f64,
    /// Python `hit_count` — number of recalls that surfaced this memory.
    pub use_count: i64,
    pub created_at: i64,
    /// Python `last_used` (unix seconds); drives the recency decay.
    pub last_used: i64,
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
    mem_type TEXT NOT NULL DEFAULT 'work_fact',
    scene TEXT NOT NULL DEFAULT '',
    paths TEXT NOT NULL DEFAULT '[]',
    tags TEXT NOT NULL DEFAULT '[]',
    importance REAL NOT NULL DEFAULT 0.5,
    use_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    last_used INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'manual'
);
CREATE INDEX IF NOT EXISTS idx_memories_cat ON memories(category);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);
CREATE TABLE IF NOT EXISTS memory_vectors (
    mem_id INTEGER PRIMARY KEY,
    dim INTEGER NOT NULL,
    vec BLOB NOT NULL
);
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
    // Stores created before `last_used` existed: add the column rather than
    // failing, so an old project DB keeps working after an upgrade.
    let has_last_used: i64 = store.conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='last_used'",
        [],
        |r| r.get(0),
    )?;
    if has_last_used == 0 {
        store
            .conn
            .execute_batch("ALTER TABLE memories ADD COLUMN last_used INTEGER NOT NULL DEFAULT 0")?;
    }
    for (name, ddl) in [
        ("mem_type", "ALTER TABLE memories ADD COLUMN mem_type TEXT NOT NULL DEFAULT 'work_fact'"),
        ("scene", "ALTER TABLE memories ADD COLUMN scene TEXT NOT NULL DEFAULT ''"),
    ] {
        let has: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name=?1",
                params![name],
                |r| r.get(0),
            )?;
        if has == 0 {
            store.conn.execute_batch(ddl)?;
        }
    }
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
        "INSERT INTO memories
         (content, content_norm, mem_type, scene, category, paths, tags, importance, created_at, source)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            content,
            norm,
            "work_fact",
            "",
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

/// Typed insert — Python's `ExtractMemory` shape (type + scene + priority).
pub fn add_typed(
    store: &Store,
    content: &str,
    mem_type: MemoryType,
    scene: &str,
    category: &str,
    paths: &[String],
    tags: &[String],
    importance: f64,
    source: &str,
) -> Result<Option<i64>> {
    let id = add(store, content, category, paths, tags, importance, source)?;
    if let Some(id) = id {
        store.conn.execute(
            "UPDATE memories SET mem_type = ?1, scene = ?2 WHERE id = ?3",
            params![mem_type.as_str(), scene, id],
        )?;
    }
    Ok(id)
}

/// Port of `MemoryStore.prune_stale`: drop memories whose **explicitly tracked**
/// paths are missing from disk.
///
/// Deliberate restraints (Python is explicit about these): content may mention
/// historical or illustrative paths, so only tracked `paths` count; a path that
/// resolves outside the project root is never grounds for deletion; and short
/// tokens without a separator are ignored.
pub fn prune_stale(store: &Store, root: &Path) -> Result<usize> {
    ensure_schema(store)?;
    let root = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf());

    let all = list_all(store, 100_000)?;
    if all.is_empty() {
        return Ok(0);
    }

    let mut stale = Vec::new();
    for m in &all {
        for raw in &m.paths {
            if raw.len() <= 4 || !raw.contains('/') {
                continue;
            }
            let p = Path::new(raw);
            let resolved = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            let resolved = resolved.canonicalize().unwrap_or(resolved);
            // Never prune a path outside this project.
            if !resolved.starts_with(&root) {
                continue;
            }
            if !resolved.exists() {
                stale.push(m.id);
                break;
            }
        }
    }

    let mut removed = 0usize;
    for id in stale {
        removed += delete(store, id)? as usize;
    }
    Ok(removed)
}

pub fn get(store: &Store, id: i64) -> Result<Option<Memory>> {
    ensure_schema(store)?;
    Ok(store
        .conn
        .query_row(
            "SELECT id, content, mem_type, scene, category, paths, tags, importance, use_count, created_at, last_used, source
             FROM memories WHERE id=?1",
            params![id],
            row_to_memory,
        )
        .optional()?)
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    // SELECT order is: id, content, mem_type, scene, category, paths, tags, ...
    let paths: String = r.get(5)?;
    let tags: String = r.get(6)?;
    let mem_type: String = r.get(2)?;
    Ok(Memory {
        id: r.get(0)?,
        content: r.get(1)?,
        mem_type: MemoryType::from_str(&mem_type),
        scene: r.get(3)?,
        category: r.get(4)?,
        paths: serde_json::from_str(&paths).unwrap_or_default(),
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        importance: r.get(7)?,
        use_count: r.get(8)?,
        created_at: r.get(9)?,
        last_used: r.get(10)?,
        source: r.get(11)?,
    })
}

pub fn list(store: &Store, limit: usize) -> Result<Vec<Memory>> {
    ensure_schema(store)?;
    let mut stmt = store.conn.prepare(
        "SELECT id, content, mem_type, scene, category, paths, tags, importance, use_count, created_at, last_used, source
         FROM memories ORDER BY importance DESC, created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], row_to_memory)?;
    Ok(rows.flatten().collect())
}

/// Every memory, newest first — the candidate set for ranking.
///
/// Ranking must consider the whole store (bounded): Python retrieves over the
/// full vector index, and pre-truncating by `importance` silently hid any low
/// priority memory from ranking entirely.
pub fn list_all(store: &Store, cap: usize) -> Result<Vec<Memory>> {
    ensure_schema(store)?;
    let mut stmt = store.conn.prepare(
        "SELECT id, content, mem_type, scene, category, paths, tags, importance, use_count, created_at, last_used, source
         FROM memories ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![cap as i64], row_to_memory)?;
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
    reinforce(store, id)
}

/// Python `MemoryStore._reinforce`: bump usage and stamp last_used so the
/// recency and usage terms in `_blended_score` actually move.
pub fn reinforce(store: &Store, id: i64) -> Result<()> {
    ensure_schema(store)?;
    store.conn.execute(
        "UPDATE memories SET use_count = use_count + 1, last_used = ?1 WHERE id=?2",
        params![now(), id],
    )?;
    Ok(())
}

/// Retrieve, then reinforce every returned memory — Python's `recall()` calls
/// `_reinforce(ids)` before returning, which is what makes `usage` a live
/// signal instead of a constant 0.
pub fn recall(
    store: &Store,
    embedder: Option<&dyn imc_embed::Embedder>,
    query: &str,
    active_paths: &[String],
    mem_type: Option<&str>,
    scene: Option<&str>,
    limit: usize,
) -> Result<Vec<MemoryHit>> {
    let hits = retrieve(store, embedder, query, active_paths, mem_type, scene, limit)?;
    for h in &hits {
        reinforce(store, h.memory.id)?;
    }
    Ok(hits)
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

/// Query tokens worth searching with: at least three characters and not a
/// common English word.
///
/// The FTS leg expands every token into `"token" OR "token"*`. A two-letter
/// word such as `is` therefore prefix-matches unrelated identifiers — `is*`
/// hits `iso`, `issue`, `item` — and one spurious hit makes the normalised
/// BM25 score 1.0, which is enough to pass the similarity cutoff and inject an
/// unrelated memory. Python's BM25 leg matches whole tokens with IDF
/// weighting, so it never saw this.
fn search_tokens(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "and", "are", "but", "can", "did", "does", "for", "from", "had", "has", "have", "how", "into",
        "its", "not", "the", "their", "them", "then", "there", "these", "they", "this", "that", "was",
        "were", "what", "when", "where", "which", "while", "who", "why", "will", "with", "you", "your",
        "use", "used", "using", "get", "got", "make", "made", "need", "want", "way", "all", "any",
        "can", "should", "would", "could", "about", "after", "before", "between", "during", "under",
    ];
    tokenize(query)
        .into_iter()
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() >= 3)
        .filter(|t| !STOP.contains(&t.as_str()))
        .collect()
}

/// Python's `SIMILARITY_CUTOFF` (`main/memory/store.py`). Candidates below it
/// are dropped before the blended re-rank, so a memory that does not match the
/// query at all is never returned — and never injected.
const SIMILARITY_CUTOFF: f64 = 0.55;

fn lexical_scores(store: &Store, query: &str, limit: usize) -> Result<HashMap<i64, f64>> {
    let tokens = search_tokens(query);
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let expr = tokens
        .iter()
        .map(|t| format!("\"{}\" OR \"{}\"*", t.replace('"', ""), t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT rowid, bm25(memories_fts) FROM memories_fts WHERE memories_fts MATCH ?1 LIMIT ?2"
    );
    let mut stmt = store.conn.prepare(&sql).context("prepare memories_fts MATCH")?;
    let rows: Vec<(i64, f64)> = stmt
        .query_map(params![expr, limit as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .flatten()
        .collect();
    Ok(rows.into_iter().map(|(id, bm)| (id, -bm)).collect())
}

/// Python `MemoryStore._blended_score`, ported faithfully:
///
///   0.55 * similarity + 0.20 * priority + 0.10 * recency
///   + 0.05 * usage     + 0.10 * overlap
///
/// - similarity: fused lexical (BM25 normalised to 0..1) and cosine. Python
///   only has a vector leg; the lexical half exists because this port can run
///   with no embedding endpoint, and BM25 carries filename/identifier matches
///   that a code-embedding cosine misses.
/// - priority:   `importance` (0..1 here == Python's 0..100 / 100).
/// - recency:    exp decay on `last_used`, ~30 day constant.
/// - usage:      log1p(hit_count)/log1p(20) — saturates around 20 recalls.
/// - overlap:    1.0 when the memory tracks a file in the caller's context.
/// Cosine floor for the vector leg.
///
/// Measured on a 41-memory store with `text-embedding-3-small`: relevant pairs
/// scored 0.71-0.82, unrelated pairs 0.09-0.28. Without a floor every pair is
/// "somewhat similar" and a nonsense query returns the highest-importance
/// filler. BM25 hits bypass the floor (an exact term match is explicit intent).
fn min_cosine() -> f64 {
    std::env::var("IMC_MEMORY_MIN_SIM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.45)
}

fn blended_score(
    similarity: f64,
    memory: &Memory,
    context_paths: &HashSet<String>,
    now: i64,
) -> f64 {
    let recency = if memory.last_used > 0 {
        let age_days = (now - memory.last_used).max(0) as f64 / 86400.0;
        (-age_days / 30.0).exp()
    } else if memory.created_at > 0 {
        let age_days = (now - memory.created_at).max(0) as f64 / 86400.0;
        (-age_days / 30.0).exp()
    } else {
        1.0
    };

    let usage = ((memory.use_count.max(0) as f64 + 1.0).ln() / 21.0_f64.ln()).min(1.0);

    let overlap = if context_paths.is_empty() {
        0.0
    } else if memory.paths.iter().any(|p| context_paths.contains(p)) {
        1.0
    } else {
        0.0
    };

    0.55 * similarity.clamp(0.0, 1.0)
        + 0.20 * memory.importance.clamp(0.0, 1.0)
        + 0.10 * recency
        + 0.05 * usage
        + 0.10 * overlap
}

/// Ranked memory retrieval over the **whole** store.
///
/// `remember`: `retrieve` alone ranks; `recall` (see above) ranks and then
/// reinforces, matching Python's `recall()`.
pub fn retrieve(
    store: &Store,
    embedder: Option<&dyn imc_embed::Embedder>,
    query: &str,
    active_paths: &[String],
    mem_type: Option<&str>,
    scene: Option<&str>,
    limit: usize,
) -> Result<Vec<MemoryHit>> {
    ensure_schema(store)?;

    // Candidates: every memory, not `limit * 6` rows pre-cut by importance.
    // The old candidate query silently hid any low-priority memory from
    // ranking, so an exact lexical hit could lose to irrelevant filler.
    const CANDIDATE_CAP: usize = 50_000;
    let all = list_all(store, CANDIDATE_CAP)?;
    if all.is_empty() {
        return Ok(Vec::new());
    }

    let lex = lexical_scores(store, query, CANDIDATE_CAP)?;

    // Cosine leg, when an embedder is configured.
    let query_vec = embedder
        .and_then(|e| e.embed(&[query.to_string()]).ok())
        .and_then(|v| v.into_iter().next());
    let mut vectors: HashMap<i64, Vec<f32>> = HashMap::new();
    if let Some(qv) = &query_vec {
        let rows = load_vectors(store)?;
        for (id, v) in rows {
            if v.len() == qv.len() {
                vectors.insert(id, v);
            }
        }
    }

    // Normalise BM25 to 0..1 over the candidates so it and cosine are on one
    // scale before fusing (raw bm25() magnitude depends on corpus size).
    let lex_max = lex.values().cloned().fold(0.0_f64, f64::max);

    let now = now();
    let context_paths: HashSet<String> = active_paths.iter().cloned().collect();
    let mut hits = Vec::new();
    for m in all {
        let bm = lex
            .get(&m.id)
            .map(|s| if lex_max > 0.0 { s / lex_max } else { 0.0 })
            .unwrap_or(0.0);
        let cos = match (vectors.get(&m.id), query_vec.as_ref()) {
            (Some(v), Some(q)) => cosine(v, q),
            _ => 0.0,
        };
        // Fuse: whichever leg is present contributes; when both do, average
        // them (BM25 rewards exact identifiers, cosine rewards paraphrase).
        let similarity = match (bm > 0.0, cos > 0.0) {
            (true, true) => (bm + cos) / 2.0,
            (true, false) => bm,
            (false, true) => cos,
            (false, false) => 0.0,
        };

        // Relevance floor, same as Python: a candidate that does not match the
        // query must not survive on its priority/recency/usage weights alone.
        // Without it every memory scored ~0.75 and the proxy injected an
        // unrelated one into every request.
        if similarity < SIMILARITY_CUTOFF {
            continue;
        }

        let score = blended_score(similarity, &m, &context_paths, now);

        let matched_paths: Vec<String> = m
            .paths
            .iter()
            .filter(|p| context_paths.contains(p.as_str()))
            .cloned()
            .collect();

        // Type / scene filter: skip memories that don't match the requested
        // work-memory type or working scene.
        if let Some(mt) = mem_type {
            if m.mem_type.as_str() != mt {
                continue;
            }
        }
        if let Some(sc) = scene {
            if m.scene != sc {
                continue;
            }
        }

        // Relevance gate, not a magic score threshold: keep a memory only when
        // it actually matched the query (lexical, vector above the floor, or
        // it tracks a file in the caller's context).
        let relevant = bm > 0.0 || cos >= min_cosine() || !matched_paths.is_empty();
        if !relevant {
            continue;
        }

        hits.push(MemoryHit {
            memory: m,
            score,
            matched_paths,
        });
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    Ok(hits)
}

fn load_vectors(store: &Store) -> Result<Vec<(i64, Vec<f32>)>> {
    let mut stmt = store
        .conn
        .prepare("SELECT mem_id, vec FROM memory_vectors WHERE mem_id IN (SELECT id FROM memories)")?;
    let rows = stmt.query_map([], |r| {
        let id: i64 = r.get(0)?;
        let blob: Vec<u8> = r.get(1)?;
        let mut v = Vec::with_capacity(blob.len() / 4);
        for c in blob.chunks_exact(4) {
            v.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
        Ok((id, v))
    })?;
    Ok(rows.flatten().collect())
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
        if let Some(rest) = t.strip_prefix("RULE:").or_else(|| t.strip_prefix("Rule:")) {
            if rest.len() > 20 {
                push(rest.trim().to_string(), "rule", "auto");
            }
        } else if let Some(rest) = t.strip_prefix("NOTE:").or_else(|| t.strip_prefix("Note:")) {
            if rest.len() > 20 {
                push(rest.trim().to_string(), "note", "auto");
            }
        } else if let Some(rest) = t.strip_prefix("CONVENTION:").or_else(|| t.strip_prefix("Convention:")) {
            if rest.len() > 20 {
                push(rest.trim().to_string(), "convention", "auto");
            }
        } else if let Some(rest) = t.strip_prefix("ALWAYS:").or_else(|| t.strip_prefix("Always:")) {
            if rest.len() > 20 {
                push(rest.trim().to_string(), "rule", "auto");
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