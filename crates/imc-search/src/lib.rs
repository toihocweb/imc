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
/// OR-only (non-conjunctive) matches rank below exact all-term matches.
const OR_PENALTY: f64 = 0.55;
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

/// Function words carry no code signal; drop them before matching.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "on", "for", "with", "is", "are", "was",
    "be", "do", "does", "did", "how", "what", "where", "which", "who", "why", "when", "that",
    "this", "it", "its", "we", "you", "i", "my", "our", "their", "from", "by", "as", "at", "can",
    "should", "would", "could", "if", "then", "than", "use", "using", "used",
];

/// Two-phase lexical retrieval: strict conjunction first, then relax to OR
/// when it cannot fill the candidate pool. `query_tokens` is the stopword-
/// filtered token list, reused for the path boost.
fn bm25_scores(store: &Store, query: &str) -> Result<HashMap<i64, f64>> {
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = "SELECT rowid, bm25(chunks_fts) FROM chunks_fts WHERE chunks_fts MATCH ?1 LIMIT ?2";

    let run = |expr: &str, cap: usize| -> Result<Vec<(i64, f64)>> {
        let mut stmt = store.conn.prepare(sql)?;
        let rows = stmt.query_map(params![expr, cap as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?;
        // bm25() is negative (lower = better); invert into a positive score.
        Ok(rows.flatten().collect())
    };

    // Always fetch both: strict AND hits, plus OR-only hits at a penalty.
    // Running OR only as a fallback starved small-but-correct files
    // (e.g. services/copilot/stream_translation.py) from the pool entirely.
    // The relaxed pass needs a wider cap: small, very specific files
    // (services/copilot/*) lose on bm25 and would never enter the pool.
    let strict = run(&join_expr(&tokens, false), CANDIDATES)?;
    let mut rows: Vec<(i64, f64)> = strict
        .iter()
        .map(|(id, bm)| (*id, -(*bm)))
        .collect();
    let relaxed = run(&join_expr(&tokens, true), CANDIDATES * 4)?;

    // The relaxed pass is an OR, so one incidental token is enough to enter it,
    // and a nonsense query still shares a rare word with the corpus — measured
    // on /root/imc: "quantum banana teleport protocol" matches only `protocol`
    // (10 chunks), "kubernetes helm chart rollout strategy" only `strategy`
    // (7), and "photosynthesis chlorophyll wavelength" nothing at all.
    //
    // The separating signal is query-level, not per-chunk: every real query in
    // the labelled set has at least two content tokens that appear *somewhere*
    // in the corpus (lowest is 2 of 3, "tokenization and bm25 scoring"), while
    // every junk query has at most one. So when fewer than two tokens exist in
    // the corpus, the query has no lexical support and the relaxed pass adds
    // nothing. Per-chunk ">= 2 tokens" gates look similar but are wrong: they
    // cost three top-3 answers, because a correct file may legitimately share
    // just one token. Strict (all-token) hits are never gated.
    let tokens_present = tokens_present_in_corpus(store, &tokens)?;
    if tokens.len() >= 2 && tokens_present < 2 {
        return Ok(rows.into_iter().collect());
    }

    for (id, bm) in relaxed {
        if !rows.iter().any(|(i, _)| *i == id) {
            rows.push((id, -bm * OR_PENALTY));
        }
    }

    let mut out = HashMap::new();
    for (id, score) in rows {
        out.insert(id, score);
    }
    Ok(out)
}

/// How many distinct query tokens the corpus knows about.
///
/// Three kinds of evidence count, because FTS5 stores `OrderService` as one
/// token: a chunk match, a symbol whose *name* matches, and a file path that
/// contains the token. The last two matter for queries like `OrderService.java`,
/// which tokenizes to `order`/`service`/`java` — only `java` is an FTS token, so
/// a text-only probe would call a real query unsupported and drop every hit.
/// Paths are checked in memory (a few hundred files) rather than with a
/// `LIKE '%tok%'` scan per token.
fn tokens_present_in_corpus(store: &Store, tokens: &[String]) -> Result<usize> {
    let paths: Vec<String> = store
        .all_file_hashes()
        .into_iter()
        .map(|(p, _)| p.to_lowercase())
        .collect();
    let mut stmt = store
        .conn
        .prepare("SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH ?1 LIMIT 1")?;
    let mut present = 0usize;
    for t in tokens.iter().take(12) {
        let mut rows = stmt.query(params![format!("\"{t}\"")])?;
        let in_chunks = rows.next()?.is_some();
        let in_symbols = !store.find_symbols_by_name(t, 1).is_empty();
        let in_paths = paths.iter().any(|p| p.contains(t.as_str()));
        if in_chunks || in_symbols || in_paths {
            present += 1;
        }
    }
    Ok(present)
}

/// Stopword-filtered tokens for one query (single source of truth).
pub fn query_tokens(raw: &str) -> Vec<String> {
    let mut tokens = tokenize(raw);
    tokens.retain(|t| !STOPWORDS.contains(&t.as_str()));
    tokens
}

/// Quoted tokens joined by `AND` (space) when strict, or by `OR` when relaxed;
/// the last (most specific) token gets a `*` prefix match in both modes.
fn join_expr(tokens: &[String], or_mode: bool) -> String {
    let mut parts: Vec<String> = tokens.iter().map(|t| format!("\"{t}\"")).collect();
    if let Some(last) = parts.last_mut() {
        // FTS5 prefix syntax is `tok*` / `"tok"*` — the star must sit OUTSIDE the
        // quotes. Writing `"tok*"` is a literal token and matches nothing, which
        // silently emptied the whole BM25 leg.
        *last = format!("{}\"*", last.trim_end_matches('"'));
    }
    if or_mode {
        parts.join(" OR ")
    } else {
        parts.join(" ")
    }
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

/// Cosine floor for the vector leg.
///
/// Measured on `/root/imc` (1234 chunks, `text-embedding-3-small`): a nonsense
/// query ("quantum banana teleport protocol") peaks at 0.26, while the correct
/// file for a real query scores 0.35-0.53. Without a floor every chunk is
/// "somewhat similar" to everything, so a nonsense query returns five confident
/// hits at ~0.016 fused — and an agent reads that as evidence. BM25 hits bypass
/// the floor: an exact term match is explicit intent (same rule as
/// `imc_memory::min_cosine`).
fn min_cosine() -> f64 {
    std::env::var("IMC_SEARCH_MIN_SIM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.30)
}

/// Floor for a query that has *some* lexical support; the vector leg is then a
/// second opinion on a query we know is about this corpus.
fn vector_scores(store: &Store, query_vec: &[f32]) -> HashMap<i64, f64> {
    vector_scores_with_floor(store, query_vec, min_cosine())
}

/// Floor when the lexical leg found nothing: the vector leg is then the *only*
/// evidence, so "somewhat similar" is not enough.
///
/// Measured over 8 nonsense + 8 real queries on two corpora (1234 and 2125
/// chunks, `/root/imc` and a Korean full-stack repo): no absolute cosine, gap or
/// z-score separates nonsense from real across both — on the Korean corpus a
/// nonsense query peaks at 0.396, above a real query's 0.333. What *does*
/// separate them is this case: nonsense queries have no lexical support at all
/// and peak at 0.396, while every query whose answer exists and has no lexical
/// support starts at 0.45. Hence a much higher bar when BM25 is empty.
fn vector_scores_with_lexical_support_absent(store: &Store, query_vec: &[f32]) -> HashMap<i64, f64> {
    vector_scores_with_floor(store, query_vec, min_cosine().max(0.45))
}

fn vector_scores_with_floor(store: &Store, query_vec: &[f32], floor: f64) -> HashMap<i64, f64> {
    let mut scored: Vec<(i64, f64)> = store
        .all_vectors()
        .into_iter()
        .filter(|(_, v)| v.len() == query_vec.len())
        .map(|(id, v)| (id, cosine(query_vec, &v)))
        .filter(|(_, s)| *s >= floor)
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

/// Jaccard similarity between two chunk texts on whitespace tokens: used by
/// MMR to avoid picking five chunks that all say the same thing.
fn chunk_similarity(a: &str, b: &str) -> f64 {
    let ta: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let tb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let inter = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// Lexical-ish boost: symbols whose name matches query tokens.
fn symbol_name_boost(store: &Store, query: &str) -> HashMap<String, f64> {
    let tokens: Vec<String> = tokenize(query)
        .into_iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect();
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
    // Both bounds must sit on a char boundary: `i + 120` above lands inside a
    // multi-byte char often enough (Korean, CJK, emoji) that `text[start..end]`
    // panicked the whole `imc search -r` run.
    let mut start = start.min(text.len().saturating_sub(max_chars));
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
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
    // `match_indices`, not a manual `text[start..].find(name)` loop: advancing by
    // one byte from a match can land inside a multi-byte char and panic on the
    // next slice (a Korean chunk did exactly that).
    for (abs, _) in text.match_indices(name) {
        let before_ok =
            abs == 0 || !(bytes[abs - 1].is_ascii_alphanumeric() || bytes[abs - 1] == b'_');
        let after = abs + name.len();
        let after_ok =
            after >= bytes.len() || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

pub struct SearchOptions {
    pub limit: usize,
    /// Enable the vector leg (needs an embedder).
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
    embedder: Option<&dyn imc_embed::Embedder>,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let mut lists: Vec<HashMap<i64, f64>> = Vec::new();

    let bm = bm25_scores(store, query)?;
    let lexical_support = !bm.is_empty();
    if lexical_support {
        lists.push(bm);
    }

    if let Some(embedder) = embedder.filter(|_| opts.vector) {
        // Same token filter as the lexical leg, else the hashing embedder
        // spends its whole budget on "how"/"to"/"the".
        let vq = imc_embed::vector_query(query);
        if let Ok(qv) = embedder.embed(&[vq]) {
            if let Some(v) = qv.into_iter().next() {
                let vs = if lexical_support {
                    vector_scores(store, &v)
                } else {
                    vector_scores_with_lexical_support_absent(store, &v)
                };
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

    let mut scored: Vec<(f64, SearchHit)> = Vec::new();
    for (chunk_id, score) in fused.into_iter().take(opts.limit * 24) {
        let Some(chunk) = store.chunk_by_id(chunk_id) else {
            continue;
        };
        let mut s = score;
        if let Some(b) = file_boost.get(&chunk.file) {
            s += b * 0.02;
        }
        // Chunk that IS the named symbol (query "deleteProject" -> its def).
        if let (Some(sym), Some(_)) = (chunk.sym_id.as_deref(), chunk.sym_id.as_deref()) {
            let short = sym.rsplit("::").next().unwrap_or("");
            // Token match covers snake_case (`spawn_engine`), but a camelCase
            // query tokenizes into pieces (`taxOn` -> tax, on) and can never
            // equal the symbol name, so also compare the collapsed query.
            // Compare with separators removed on BOTH sides, so the query
            // matches however the symbol is cased or punctuated:
            //   `blended_score` -> blendedscore == blendedscore
            //   `taxOn`         -> taxon        == taxon
            //   `spawnEngine`   -> spawnengine  == spawnengine
            let norm = |s: &str| -> String {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect()
            };
            let collapsed = norm(query);
            // Both checks matter: a query token that is itself a symbol name
            // (`... weights for score ...` -> `score`) and a query that *is*
            // the symbol name in any casing/punctuation.
            let token_hit = query_tokens(query).iter().any(|t| t.eq_ignore_ascii_case(short));
            if token_hit || (!collapsed.is_empty() && collapsed == norm(short)) {
                s += 0.040;
            }
        }
        // A query token that appears in the path is strong evidence:
        // "parser" -> main/ingest/parser.py, "limits" -> main/search/limits.py.
        let path_lc = chunk.file.to_lowercase();
        let stem_lc = path_lc
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        for t in query_tokens(query) {
            if t.len() < 3 {
                continue;
            }
            // Filename match (`git.py` for "git") beats directory match.
            if stem_lc.contains(&t) {
                s += 0.030;
            } else if path_lc.contains(&t) {
                s += 0.012;
            }
        }
        // Production code is more useful than test code for answering "how
        // does it work" questions; test files dominate top-K when the query
        // matches test assertions. Penalize test files unless the query specifically
        // targets tests, and reward src/main/.
        let is_test = path_lc.contains("/src/test/")
            || path_lc.contains("/tests/")
            || path_lc.contains("/test/")
            || path_lc.contains("_test.")
            || path_lc.contains("test_")
            || path_lc.contains("_test.rs")
            || path_lc.contains("_test.py")
            || path_lc.contains(".spec.")
            || path_lc.contains(".cy.")
            || path_lc.contains("/cypress/")
            || path_lc.contains("/e2e/")
            || path_lc.ends_with("test.java")
            || path_lc.ends_with("tests.java")
            || path_lc.ends_with("testcase.java");
        let query_lc = query.to_lowercase();
        let query_wants_test = query_lc.contains("test")
            || query_lc.contains("spec")
            || query_lc.contains("mock")
            || query_lc.contains("assert")
            || query_lc.contains("cypress");
        let is_main = path_lc.contains("/src/main/");
        if is_test && !query_wants_test {
            s *= 0.5;
        } else if is_main && !query_wants_test {
            s += 0.010;
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
        scored.push((
            s,
            SearchHit {
            score: s,
            file: chunk.file.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            snippet: snippet(&chunk.text, query, 600),
            text,
            sym_name,
            sym_kind,
            source: "hybrid",
            },
        ));
    }

    // MMR: pick the highest-scored chunk first, then greedily pick the chunk
    // that maximizes lambda * relevance - (1-lambda) *
    // max_similarity_with_selected. This prevents the top-5 from being
    // dominated by chunks from the same file (e.g. three chunks from
    // RefreshTokenRotationTest.java) and gives production code a chance to
    // appear.
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    // The relevance scores are RRF values in the 0.01-0.06 range, while the
    // MMR diversity penalty is 0..1. On the original scale the penalty
    // dominated every score difference, so the order was effectively
    // "diverse first, relevant second" — a chunk with score 0.0296 lost to
    // one with 0.0231. Normalise the scores to 0..1 first, then the two terms
    // are comparable and the diversity trade-off is the one that was intended.
    let max_score = scored.first().map(|(s, _)| *s).unwrap_or(0.0);
    if max_score > 0.0 {
        for (s, _) in scored.iter_mut() {
            *s /= max_score;
        }
    }
    let lambda: f64 = 0.70;
    let mut selected: Vec<SearchHit> = Vec::new();
    let mut remaining: Vec<(f64, SearchHit)> = scored;
    while !remaining.is_empty() && selected.len() < opts.limit {
        let mut best_idx: usize = 0;
        let mut best_mmr: f64 = f64::NEG_INFINITY;
        for (i, (score, hit)) in remaining.iter().enumerate() {
            let sim_to_selected = selected
                .iter()
                .map(|s| chunk_similarity(&hit.text, &s.text))
                .fold(0.0f64, f64::max);
            let mmr = lambda * (*score) - (1.0 - lambda) * sim_to_selected;
            if mmr > best_mmr {
                best_mmr = mmr;
                best_idx = i;
            }
        }
        let (score, mut hit) = remaining.remove(best_idx);
        // Report the ranking key that was actually used, so the printed order
        // and the printed score agree.
        hit.score = score;
        selected.push(hit);
    }
    Ok(selected)
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use imc_core::store;

    fn tmp_db(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("imc-search-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// Two files with disjoint vocabulary: `billing.py` knows invoice/tax words,
    /// `crypto.py` knows the word "protocol" (the trap a junk query lands on).
    fn seeded(tag: &str) -> (store::Store, std::path::PathBuf) {
        let db = tmp_db(tag);
        let store = store::open(&db).unwrap();
        // `chunks_fts` is created by `imc index`, not by `store::open`.
        ensure_fts(&store).unwrap();
        store.upsert_file("src/billing.py", "h1", Some("python"), 64).unwrap();
        store.upsert_file("src/crypto.py", "h2", Some("python"), 64).unwrap();
        store
            .insert_chunk(
                "src/billing.py",
                1,
                3,
                "src/billing.py:1-3\ndef calculate_tax(amount, rate):\n    return amount * rate\n",
                None,
            )
            .unwrap();
        store
            .insert_chunk(
                "src/crypto.py",
                20,
                22,
                "src/crypto.py:20-22\n# TLS protocol handshake notes\ndef handshake():\n    return None\n",
                None,
            )
            .unwrap();
        (store, db)
    }

    /// The vector floor is raised when the lexical leg found nothing: with no
    /// lexical support the vector leg is the only evidence, and "somewhat
    /// similar" (0.30-0.45) is not evidence. Measured: nonsense queries max out
    /// at 0.396 across two corpora, while answerable-but-unlexical queries
    /// start at 0.45.
    #[test]
    fn the_vector_floor_is_higher_without_lexical_support() {
        let (store, db) = seeded("floors");
        let ids: Vec<i64> = store.all_vectors().iter().map(|(id, _)| *id).collect();
        assert!(ids.is_empty(), "seeded store starts without vectors");
        let a = store
            .insert_chunk("src/billing.py", 10, 12, "src/billing.py:10-12\ncalculate_tax\n", None)
            .unwrap();
        let b = store
            .insert_chunk("src/crypto.py", 30, 32, "src/crypto.py:30-32\nhandshake protocol\n", None)
            .unwrap();
        let q = vec![1.0f32, 0.0, 0.0, 0.0];
        store.insert_embedding(a, 4, &[1.0, 0.0, 0.0, 0.0]).unwrap(); // cosine 1.0
        store.insert_embedding(b, 4, &[0.35, 0.93, 0.0, 0.0]).unwrap(); // cosine ~0.35

        let normal = vector_scores_with_floor(&store, &q, min_cosine());
        assert!(normal.contains_key(&a) && normal.contains_key(&b), "{normal:?}");

        let no_lexical = vector_scores_with_floor(&store, &q, min_cosine().max(0.45));
        assert!(no_lexical.contains_key(&a), "a 1.0 cosine survives any floor");
        assert!(
            !no_lexical.contains_key(&b),
            "a 0.35 cosine must not be the sole evidence for a query the corpus never mentions"
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn real_query_returns_its_chunk() {
        let (store, db) = seeded("real");
        let hits = search(&store, None, "calculate tax rate", &SearchOptions { limit: 5, vector: false }).unwrap();
        assert!(!hits.is_empty(), "a two-token query in the corpus must hit");
        assert_eq!(hits[0].file, "src/billing.py");
        let _ = std::fs::remove_file(&db);
    }

    /// Measured on /root/imc: junk shares one word with the corpus and used to
    /// come back as five confident hits. The gate must keep it empty.
    #[test]
    fn junk_query_returns_nothing() {
        let (store, db) = seeded("junk");
        for q in [
            "quantum banana teleport protocol",
            "kubernetes helm chart rollout strategy",
            "photosynthesis chlorophyll wavelength",
        ] {
            let hits = search(&store, None, q, &SearchOptions { limit: 5, vector: false }).unwrap();
            assert!(hits.is_empty(), "{q:?} returned {hits:?}");
        }
        let _ = std::fs::remove_file(&db);
    }

    /// A query naming a file, with the extension, still has support: the tokens
    /// are not FTS words (`calculate_tax` is one token), only path evidence.
    #[test]
    fn filename_query_is_not_gated_out() {
        let (store, db) = seeded("filename");
        let hits = search(&store, None, "billing.py", &SearchOptions { limit: 5, vector: false }).unwrap();
        assert!(!hits.is_empty(), "a file named in the query must still hit");
        let _ = std::fs::remove_file(&db);
    }

    /// MMR keeps the top-5 from being dominated by one file: three chunks from
    /// the same file should collapse into one, letting a different file enter
    /// the top-5.
    /// Test files (src/test/...) are penalized relative to src/main/ so a
    /// production file can win the top-5 over an equally-matching test file.
    #[test]
    fn test_files_are_penalized_vs_main() {
        let (store, db) = seeded("penalty");
        // Same textual content, different path.
        store
            .insert_chunk("src/main/lib.rs", 1, 3, "calculate tax rate amount
", None)
            .unwrap();
        store
            .insert_chunk("src/test/lib_test.rs", 1, 3, "calculate tax rate amount
", None)
            .unwrap();
        let hits = search(&store, None, "tax rate amount", &SearchOptions { limit: 2, vector: false }).unwrap();
        assert!(!hits.is_empty(), "{hits:?}");
        // The main file should rank at or above the test file.
        let main_idx = hits.iter().position(|h| h.file == "src/main/lib.rs");
        let test_idx = hits.iter().position(|h| h.file == "src/test/lib_test.rs");
        match (main_idx, test_idx) {
            (Some(m), Some(t)) => assert!(m <= t, "main.rs should outrank test.rs: {hits:?}"),
            (Some(_), None) => {} // only main surfaced, fine
            (None, Some(_)) => panic!("test.rs outranked main.rs: {hits:?}"),
            (None, None) => panic!("nothing surfaced: {hits:?}"),
        }
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn single_token_query_is_exempt_from_the_gate() {
        let (store, db) = seeded("single");
        let hits = search(&store, None, "handshake", &SearchOptions { limit: 5, vector: false }).unwrap();
        assert!(!hits.is_empty(), "a one-word query must not be gated");
        let _ = std::fs::remove_file(&db);
    }
}
