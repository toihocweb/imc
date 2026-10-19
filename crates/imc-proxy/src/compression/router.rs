//! HTTP-facing compression endpoints: /v1/compress, /v1/retrieve, /v1/stats.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::compression::{ccr_store::CcrStore, compress};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct CompressRequest {
    pub content: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CompressResponse {
    pub id: String,
    pub kind: String,
    pub original: String,
    pub compressed: String,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub saved_tokens: usize,
    pub compression_ratio: f64,
}

#[derive(Debug, Deserialize)]
pub struct RetrieveRequest {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct RetrieveResponse {
    pub id: String,
    pub original: String,
    pub compressed: String,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_originals: i64,
    pub total_original_tokens: i64,
    pub total_compressed_tokens: i64,
    pub total_saved_tokens: i64,
    pub compression_ratio: f64,
}

pub struct CompressionState {
    pub db_path: PathBuf,
}

pub fn router(db_path: PathBuf) -> Router {
    let state = Arc::new(CompressionState { db_path });
    Router::new()
        .route("/v1/compress", post(compress_handler))
        .route("/v1/retrieve", post(retrieve_handler))
        .route("/v1/stats", get(stats_handler))
        .with_state(state)
}

async fn compress_handler(
    State(state): State<Arc<CompressionState>>,
    _headers: HeaderMap,
    Json(req): Json<CompressRequest>,
) -> Json<CompressResponse> {
    let result = compress(&req.content);
    let kind_name = match result.kind {
        crate::compression::ContentKind::JsonArray => "json_array",
        crate::compression::ContentKind::JsonObject => "json_object",
        crate::compression::ContentKind::SourceCode => "source_code",
        crate::compression::ContentKind::BuildLog => "build_log",
        crate::compression::ContentKind::GitDiff => "git_diff",
        crate::compression::ContentKind::PlainText => "plain_text",
        crate::compression::ContentKind::Unknown => "unknown",
    };
    let mut store = CcrStore::open(&state.db_path).expect("open ccr store");
    let id = store
        .store(
            kind_name,
            &result.original,
            &result.compressed,
            result.original_tokens as i64,
            result.compressed_tokens as i64,
        )
        .expect("store ccr");
    Json(CompressResponse {
        id,
        kind: kind_name.to_string(),
        original: result.original,
        compressed: result.compressed,
        original_tokens: result.original_tokens,
        compressed_tokens: result.compressed_tokens,
        saved_tokens: result.saved_tokens,
        compression_ratio: result.compression_ratio,
    })
}

async fn retrieve_handler(
    State(state): State<Arc<CompressionState>>,
    Json(req): Json<RetrieveRequest>,
) -> Json<RetrieveResponse> {
    let store = CcrStore::open(&state.db_path).expect("open ccr store");
    let original = store.get(&req.id).unwrap_or(None);
    let compressed: String = store
        .conn
        .query_row(
            "SELECT compressed FROM ccr_originals WHERE id = ?1",
            [&req.id],
            |r| r.get(0),
        )
.unwrap_or_else(|_| String::new());
    Json(RetrieveResponse {
        id: req.id,
        original: original.unwrap_or_default(),
        compressed,
    })
}

async fn stats_handler(
    State(state): State<Arc<CompressionState>>,
) -> Json<StatsResponse> {
    let store = CcrStore::open(&state.db_path).expect("open ccr store");
    let total = store.count().unwrap_or(0);
    let entries = store.list(None, 10000).unwrap_or_default();
    let total_orig: i64 = entries.iter().map(|e| e.orig_tokens).sum();
    let total_comp: i64 = entries.iter().map(|e| e.comp_tokens).sum();
    let total_saved = total_orig - total_comp;
    let ratio = if total_orig > 0 {
        total_comp as f64 / total_orig as f64
    } else {
        1.0
    };
    Json(StatsResponse {
        total_originals: total,
        total_original_tokens: total_orig,
        total_compressed_tokens: total_comp,
        total_saved_tokens: total_saved,
        compression_ratio: ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_response_fields() {
        let r = compress(r#"[{"a":1},{"a":1}]"#);
        assert!(r.original_tokens > 0);
        assert!(r.saved_tokens <= r.original_tokens);
    }
}
