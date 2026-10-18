//! Embedding providers. Uses an OpenAI-compatible `/embeddings` endpoint when
//! `IMC_EMBED_API_KEY`/`OPENAI_API_KEY` is present, and falls back to a
//! deterministic local hashing embedder so indexing still works offline
//! (keyword-quality retrieval instead of semantic).

use anyhow::{anyhow, Context, Result};
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
        let mut out: Vec<(usize, Vec<f32>)> = Vec::with_capacity(texts.len());

        // Keep request bodies modest (OpenAI caps at 2048 inputs).
        for batch in texts.chunks(128) {
            let payload = serde_json::json!({ "model": self.cfg.model, "input": batch });
            let resp = ureq::post(&url)
                .header("Authorization", format!("Bearer {key}"))
                .header("Content-Type", "application/json")
                .send_json(payload)
                .with_context(|| format!("POST {url}"))?;
            let parsed: EmbeddingResponse = resp
                .into_body()
                .read_json::<EmbeddingResponse>()
                .context("decode embeddings response")?;
            for item in parsed.data {
                out.push((item.index + out.len(), item.embedding));
            }
        }
        out.sort_by_key(|(i, _)| *i);
        Ok(out.into_iter().map(|(_, v)| v).collect())
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