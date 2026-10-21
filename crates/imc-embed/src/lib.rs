//! Embedding providers. Uses an OpenAI-compatible `/embeddings` endpoint when
//! `IMC_EMBED_API_KEY`/`OPENAI_API_KEY` is present, and falls back to a
//! deterministic local hashing embedder so indexing still works offline
//! (keyword-quality retrieval instead of semantic).

use anyhow::{anyhow, Result};
use rayon::prelude::*;
use serde::Deserialize;

use imc_core::config::EmbedConfig;

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
}

impl RemoteEmbedder {
    pub fn new(cfg: EmbedConfig) -> Self {
        Self { cfg }
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
/// well above the core count — but not unbounded, or the endpoint rate-limits
/// and every request comes back 429.
fn concurrency() -> usize {
    std::env::var("IMC_EMBED_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(8)
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

/// One request; `Err` carries a human-readable reason including the body.
fn post_batch(
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    model: &str,
    batch: &[String],
) -> std::result::Result<Vec<Vec<f32>>, String> {
    let payload = serde_json::json!({ "model": model, "input": batch });
    let mut resp = agent
        .post(url)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| e.to_string())?;

    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let body: String = body.chars().take(300).collect();
        return Err(if body.trim().is_empty() {
            format!("HTTP {status} (empty body)")
        } else {
            format!("HTTP {status}: {body}")
        });
    }

    let parsed: EmbeddingResponse = resp
        .body_mut()
        .read_json::<EmbeddingResponse>()
        .map_err(|e| format!("decode embeddings response: {e}"))?;
    // `index` is authoritative; some gateways return items out of order.
    let mut out: Vec<Option<Vec<f32>>> = vec![None; batch.len()];
    for item in parsed.data {
        if item.index < out.len() {
            out[item.index] = Some(item.embedding);
        }
    }
    Ok(out.into_iter().map(|v| v.unwrap_or_default()).collect())
}

/// Embed with bisecting retry: a rejected batch splits and retries down to
/// single inputs, so one pathological chunk cannot abort indexing the rest.
/// Failed inputs come back as empty vectors (the caller skips those).
fn embed_with_retry(
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    model: &str,
    batch: &[String],
) -> Result<Vec<Vec<f32>>> {
    match post_batch(agent, url, key, model, batch) {
        Ok(v) => Ok(v),
        Err(_) if batch.len() > 1 => {
            let mid = batch.len() / 2;
            let (a, b) = batch.split_at(mid);
            let mut left = embed_with_retry(agent, url, key, model, a)?;
            left.append(&mut embed_with_retry(agent, url, key, model, b)?);
            Ok(left)
        }
        Err(e) => {
            eprintln!("[imc-embed] skipping 1 input: {e}");
            Ok(vec![Vec::new()])
        }
    }
}

impl Embedder for RemoteEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let key = self
            .cfg
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("missing embedding API key"))?;
        let url = format!("{}/embeddings", self.cfg.base_url.trim_end_matches('/'));
        let agent = agent();

        let clean: Vec<String> = texts.iter().map(|t| sanitize(t)).collect();

        // Batches run concurrently on their own pool: the work is waiting on
        // the network, so the limit is how many requests the endpoint
        // tolerates, not how many cores the machine has.
        let batches: Vec<&[String]> = clean.chunks(batch_size()).collect();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(concurrency().min(batches.len().max(1)))
            .build()
            .map_err(|e| anyhow!("cannot build the embedding pool: {e}"))?;
        let results: Vec<Result<Vec<Vec<f32>>>> = pool.install(|| {
            batches
                .par_iter()
                .map(|b| embed_with_retry(&agent, &url, key, &self.cfg.model, b))
                .collect()
        });

        let mut out = Vec::with_capacity(clean.len());
        for r in results {
            out.extend(r?);
        }
        Ok(out)
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
    if cfg.api_key.is_some() {
        return Some(Box::new(RemoteEmbedder::new(cfg.clone())));
    }
    if std::env::var("IMC_VECTOR").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false) {
        return Some(Box::new(HashingEmbedder::new(cfg.fallback_dim)));
    }
    None
}