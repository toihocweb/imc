//! Embedding providers. Uses an OpenAI-compatible `/embeddings` endpoint when
//! `IMC_EMBED_API_KEY`/`OPENAI_API_KEY` is present, and falls back to a
//! deterministic local hashing embedder so indexing still works offline
//! (keyword-quality retrieval instead of semantic).

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use rayon::prelude::*;
use serde::Deserialize;

use imc_core::config::EmbedConfig;

pub const STOPWORDS: &[&str] = &[
    "a", "an", "the", "in", "on", "at", "to", "for", "of", "with", "by", "from",
    "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do",
    "does", "did", "how", "what", "where", "when", "why", "which", "who", "whom",
    "this", "that", "these", "those", "it", "its", "can", "could", "should", "would",
];

/// Normalize a search query for vector embedding: splits into tokens, drops
/// common grammatical stopwords, and joins with space.
pub fn vector_query(query: &str) -> String {
    let toks: Vec<String> = tokenize(query)
        .into_iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect();
    if !toks.is_empty() {
        toks.join(" ")
    } else {
        query.to_string()
    }
}

pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn name(&self) -> String;
    fn dim(&self) -> usize;
}

/// Deterministic feature-hashing embedder: token + character trigram hashing,
/// L2-normalised. Works offline, stable across runs.
pub struct HashingEmbedder {
    dim: usize,
}

impl HashingEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(32) }
    }

    fn token_indices(&self, token: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let bytes = token.as_bytes();
        if bytes.is_empty() {
            return out;
        }
        out.push(fnv1a(token) % self.dim);
        // Character trigrams inside identifiers/words.
        let chars: Vec<char> = token.chars().collect();
        if chars.len() >= 3 {
            for i in 0..=chars.len() - 3 {
                let gram: String = chars[i..i + 3].iter().collect();
                out.push(fnv1a(&gram) % self.dim);
            }
        }
        out
    }
}

impl Embedder for HashingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .par_iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for tok in tokenize(t) {
                    for idx in self.token_indices(&tok) {
                        v[idx] += 1.0;
                    }
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }

    fn name(&self) -> String {
        format!("hashing-{}", self.dim)
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

fn fnv1a(s: &str) -> usize {
    let mut h: usize = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as usize;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Splits on non-alphanumerics, camelCase and snake_case boundaries.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
            prev_lower = c.is_lowercase() || c.is_numeric();
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 1 || w.chars().any(|c| c.is_numeric()))
        .collect()
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
    index: usize,
}

pub struct RemoteEmbedder {
    cfg: EmbedConfig,
    cache: Mutex<HashMap<String, Vec<f32>>>,
}

impl RemoteEmbedder {
    pub fn new(cfg: EmbedConfig) -> Self {
        Self {
            cfg,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

/// Max chars per input. OpenAI-compatible endpoints cap at ~8k tokens and many
/// proxies hard-fail with a bare 400 above that; truncating keeps a long code
/// chunk embeddable instead of killing the whole index run.
const MAX_INPUT_CHARS: usize = 6_000;

/// Inputs sent per request.
///
/// The default was 64, but the batch was never full: the indexer called
/// `embed()` once per file, so a request carried one file's chunks. Smaller
/// batches mean more requests that can run at the same time, and smaller
/// payloads per request.
fn batch_size() -> usize {
    std::env::var("IMC_EMBED_BATCH")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(32)
}

/// How many embedding requests may be in flight at once.
///
/// Embedding is network-bound, not CPU-bound, so the useful concurrency is
/// above the core count — but the endpoint decides the real number, and a
/// gateway that allows four at a time answers 429 to the fifth. Four is the
/// default; the limiter lowers it further when the endpoint says so.
fn concurrency() -> usize {
    std::env::var("IMC_EMBED_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4)
}

/// ureq 3 maps non-2xx to `Error::StatusCode(u16)` and throws the body away,
/// which is exactly the information needed to debug a 400. Build an Agent that
/// returns responses instead of errors so we can read the server's message.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent()
}

/// Trim + cap an input. OpenAI-compatible endpoints cap at ~8k tokens and many
/// proxies hard-fail above that; truncating keeps a long chunk embeddable
/// instead of killing the whole index run. Empty input is rejected by the API.
fn sanitize(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() {
        return " ".to_string();
    }
    if t.chars().count() > MAX_INPUT_CHARS {
        let cut: String = t.chars().take(MAX_INPUT_CHARS).collect();
        return format!("{cut}\n[truncated]");
    }
    t.to_string()
}

/// Why a batch failed.
enum BatchError {
    /// The endpoint says it is busy and when to come back.
    RateLimited { retry_after: u64, detail: String },
    /// The request itself was refused.
    Rejected(String),
}

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BatchError::RateLimited { detail, .. } => write!(f, "{detail}"),
            BatchError::Rejected(m) => write!(f, "{m}"),
        }
    }
}

/// How many embedding requests may be in flight, and how that number reacts
/// to a rate limit.
///
/// The endpoint is shared, and it says when it is busy. A fixed pool of eight
/// workers got a 429 on the first run against a gateway that allows four at
/// a time, and the input that was refused lost its vector. So the limit is
/// state: a 429 halves it, and it climbs back one at a time after a run of
/// successes.
pub struct RateLimiter {
    limit: std::sync::atomic::AtomicUsize,
    active: std::sync::atomic::AtomicUsize,
    consecutive_ok: std::sync::atomic::AtomicUsize,
}

impl RateLimiter {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: std::sync::atomic::AtomicUsize::new(limit.max(1)),
            active: std::sync::atomic::AtomicUsize::new(0),
            consecutive_ok: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Wait until a slot is free, then take it.
    pub fn acquire(&self) {
        loop {
            let limit = self.limit.load(std::sync::atomic::Ordering::Relaxed);
            let active = self.active.load(std::sync::atomic::Ordering::Relaxed);
            if active < limit {
                // Reserve the slot, but only if nobody took it meanwhile.
                if self
                    .active
                    .compare_exchange(
                        active,
                        active + 1,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    pub fn release(&self) {
        self.active
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// The endpoint is busy: halve the width.
    pub fn on_rate_limit(&self) {
        self.consecutive_ok
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let current = self.limit.load(std::sync::atomic::Ordering::Relaxed);
        let next = (current / 2).max(1);
        self.limit.store(next, std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "[imc-embed] rate limited — the concurrency drops to {next} request(s) in flight"
        );
    }

    /// A success: after a run of them, allow one more request in flight.
    pub fn on_success(&self) {
        let ok = self
            .consecutive_ok
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if ok % 32 == 0 {
            let current = self.limit.load(std::sync::atomic::Ordering::Relaxed);
            let max = concurrency();
            if current < max {
                self.limit.store(current + 1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

/// One request; `Err` carries a human-readable reason including the body.
fn post_batch(
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    model: &str,
    batch: &[String],
) -> std::result::Result<Vec<Vec<f32>>, BatchError> {
    let payload = serde_json::json!({ "model": model, "input": batch });
    let mut resp = agent
        .post(url)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| BatchError::Rejected(e.to_string()))?;

    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        // A rate limit is not a bad request: it says when to come back, and
        // the caller waits that long instead of splitting the batch (which
        // would make more requests and rate-limit harder).
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let body: String = body.chars().take(300).collect();
        if status == 429 {
            return Err(BatchError::RateLimited {
                retry_after: retry_after.unwrap_or(1).clamp(1, 60),
                detail: if body.trim().is_empty() {
                    "HTTP 429".to_string()
                } else {
                    format!("HTTP 429: {body}")
                },
            });
        }
        return Err(BatchError::Rejected(if body.trim().is_empty() {
            format!("HTTP {status} (empty body)")
        } else {
            format!("HTTP {status}: {body}")
        }));
    }

    let parsed: EmbeddingResponse = resp
        .body_mut()
        .read_json::<EmbeddingResponse>()
        .map_err(|e| BatchError::Rejected(format!("decode embeddings response: {e}")))?;
    // `index` is authoritative; some gateways return items out of order.
    let mut out: Vec<Option<Vec<f32>>> = vec![None; batch.len()];
    for item in parsed.data {
        if item.index < out.len() {
            out[item.index] = Some(item.embedding);
        }
    }
    Ok(out.into_iter().map(|v| v.unwrap_or_default()).collect())
}

/// Embed one batch, with the retry the failure asks for.
///
/// A rate limit is waited out: the endpoint says how long, and splitting the
/// batch would only make more requests. A refusal is bisected instead, so
/// one pathological chunk cannot abort indexing the rest. An input that still
/// fails comes back as an empty vector (the caller stores the chunk without
/// a vector rather than dropping the chunk).
fn embed_with_retry(
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    model: &str,
    batch: &[String],
    limiter: &RateLimiter,
) -> Result<Vec<Vec<f32>>> {
    const RATE_LIMIT_ATTEMPTS: u32 = 5;
    let mut attempt = 0u32;
    loop {
        limiter.acquire();
        let result = post_batch(agent, url, key, model, batch);
        limiter.release();
        match result {
            Ok(v) => {
                limiter.on_success();
                return Ok(v);
            }
            Err(BatchError::RateLimited { retry_after, detail }) => {
                limiter.on_rate_limit();
                attempt += 1;
                if attempt >= RATE_LIMIT_ATTEMPTS {
                    eprintln!(
                        "[imc-embed] giving up on {} input(s) after {attempt} rate limits: {detail}",
                        batch.len()
                    );
                    return Ok(vec![Vec::new(); batch.len()]);
                }
                eprintln!(
                    "[imc-embed] rate limited — waiting {retry_after}s and retrying (attempt {attempt})"
                );
                std::thread::sleep(std::time::Duration::from_secs(retry_after));
            }
            Err(BatchError::Rejected(e)) => {
                if batch.len() > 1 {
                    let mid = batch.len() / 2;
                    let (a, b) = batch.split_at(mid);
                    let mut left = embed_with_retry(agent, url, key, model, a, limiter)?;
                    left.append(&mut embed_with_retry(agent, url, key, model, b, limiter)?);
                    return Ok(left);
                }
                eprintln!("[imc-embed] skipping 1 input: {e}");
                return Ok(vec![Vec::new()]);
            }
        }
    }
}

impl Embedder for RemoteEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let clean: Vec<String> = texts.iter().map(|t| sanitize(t)).collect();

        // Query Vector Cache: check in-memory cache first to avoid redundant network calls
        let mut results: Vec<Option<Vec<f32>>> = vec![None; clean.len()];
        let mut missing_indices: Vec<usize> = Vec::new();
        let mut missing_texts: Vec<String> = Vec::new();

        if let Ok(guard) = self.cache.lock() {
            for (i, t) in clean.iter().enumerate() {
                if let Some(vec) = guard.get(t) {
                    results[i] = Some(vec.clone());
                } else {
                    missing_indices.push(i);
                    missing_texts.push(t.clone());
                }
            }
        } else {
            missing_indices = (0..clean.len()).collect();
            missing_texts = clean.clone();
        }

        if missing_texts.is_empty() {
            return Ok(results.into_iter().map(|v| v.unwrap_or_default()).collect());
        }

        let key = self
            .cfg
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("missing embedding API key"))?;
        let url = format!("{}/embeddings", self.cfg.base_url.trim_end_matches('/'));
        let agent = agent();

        // Batches run concurrently on their own pool: the work is waiting on
        // the network, so the limit is how many requests the endpoint
        // tolerates, not how many cores the machine has.
        let batches: Vec<&[String]> = missing_texts.chunks(batch_size()).collect();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency().min(batches.len().max(1)))
            .build()
            .map_err(|e| anyhow!("cannot build the embedding pool: {e}"))?;
        let limiter = RateLimiter::new(concurrency());
        let fetched_batches: Vec<Result<Vec<Vec<f32>>>> = pool.install(|| {
            batches
                .par_iter()
                .map(|b| embed_with_retry(&agent, &url, key, &self.cfg.model, b, &limiter))
                .collect()
        });

        let mut fetched = Vec::with_capacity(missing_texts.len());
        for r in fetched_batches {
            fetched.extend(r?);
        }

        // Populate cache with newly fetched vectors
        if let Ok(mut guard) = self.cache.lock() {
            if guard.len() > 1024 {
                guard.clear();
            }
            for (idx, vec) in missing_indices.iter().zip(fetched.into_iter()) {
                guard.insert(clean[*idx].clone(), vec.clone());
                results[*idx] = Some(vec);
            }
        }

        Ok(results.into_iter().map(|v| v.unwrap_or_default()).collect())
    }

    fn name(&self) -> String {
        self.cfg.model.clone()
    }

    fn dim(&self) -> usize {
        0
    }
}

/// Embedding provider selection:
///   1. remote OpenAI-compatible endpoint when an API key is present
///   2. offline hashing embedder ONLY when `IMC_VECTOR=1` (or IMC_EMBED_MODEL is
///      explicitly set while a key exists but the endpoint is unreachable)
///   3. `None` — retrieval then runs BM25 + graph only.
///
/// The hashing embedder is a lexical stand-in: measured on the `imc` repo it
/// *lowered* top-1 accuracy vs BM25-only, so it must never be on by default.
pub fn build(cfg: &EmbedConfig) -> Option<Box<dyn Embedder>> {
    let m = cfg.model.trim().to_lowercase();
    if m.is_empty() || m == "none" || m == "disabled" || m == "off" || m == "false" {
        return None;
    }
    if cfg.api_key.is_some() {
        return Some(Box::new(RemoteEmbedder::new(cfg.clone())));
    }
    if std::env::var("IMC_VECTOR").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false) {
        return Some(Box::new(HashingEmbedder::new(cfg.fallback_dim)));
    }
    None
}