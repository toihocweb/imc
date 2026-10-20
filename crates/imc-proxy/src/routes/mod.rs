//! HTTP routing for the proxy. Port target: `main/proxy/main.py` + `routes/*`.
//! `serve` is called by `imc_proxy::run` and by `imc claude`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Router;
use futures::StreamExt;

pub mod chat;
pub use crate::compression;
pub mod mcp;
pub mod messages;
pub mod models;
pub mod system;

// ---------------------------------------------------------------------------
// Shared response helpers
// ---------------------------------------------------------------------------

pub(crate) fn header_map_from_btree(headers: &std::collections::BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(k.as_bytes()),
            axum::http::HeaderValue::from_str(v),
        ) {
            map.insert(name, value);
        }
    }
    map
}

/// Port of `lib/error.forward_generic_error` for handler-level failures.
pub(crate) fn error_response(status: u16, message: &str) -> Response {
    let json = serde_json::json!({"error": {"message": message, "type": "error"}});
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(json.to_string()))
        .expect("error response")
}

fn with_headers(
    mut builder: axum::http::response::Builder,
    headers: &HeaderMap,
) -> axum::http::response::Builder {
    for (name, value) in headers.iter() {
        builder = builder.header(name, value);
    }
    builder
}

/// Relay an upstream reply.
///
/// * `stream = true` -> the body is forwarded chunk-for-chunk through
///   [`axum::body::Body::from_stream`], so SSE frames reach the client the
///   instant upstream produces them (no buffering of the whole body).
/// * `stream = false` -> buffered, then relaid verbatim.
///
/// Usage harvested from a side buffer is logged after the stream ends.
pub(crate) async fn relay(
    upstream: reqwest::Response,
    model: String,
    wants_stream: bool,
    endpoint: &'static str,
) -> Response {
    let status = upstream.status().as_u16();
    let headers = crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());

    if wants_stream {
        let log = imc_proxy_types::logger::create_handler_logger("main");
        let mut stream = upstream.bytes_stream();
        let body_stream = async_stream::stream! {
            let mut pending: Vec<u8> = Vec::new();
            let mut usage = serde_json::Map::new();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        yield Ok::<_, std::io::Error>(chunk.clone());
                        pending.extend_from_slice(&chunk);
                        harvest_sse_usage(&mut pending, &mut usage);
                    }
                    Err(e) => {
                        log.error(&format!("[Proxy] upstream stream error: {e}"));
                        break;
                    }
                }
            }
            if !usage.is_empty() {
                let _ = imc_proxy_types::usage_logger::log_usage(
                    &model,
                    endpoint,
                    &serde_json::Value::Object(usage),
                    None,
                )
                .await;
            }
        };
        let builder = with_headers(
            Response::builder().status(status).header("content-type", "text/event-stream"),
            &headers,
        );
        return builder.body(Body::from_stream(body_stream)).expect("streaming response");
    }

    let bytes = upstream.bytes().await.unwrap_or_default();
    let builder = with_headers(Response::builder().status(status), &headers);
    builder.body(Body::from(bytes)).expect("response")
}

/// Consume whole `data: {...}` lines out of `pending`, accumulating `usage`
/// fields (port of the side-buffer block in Python's `stream_generator`).
/// Partial lines are left in `pending` for the next chunk.
pub(crate) fn harvest_sse_usage(pending: &mut Vec<u8>, usage: &mut serde_json::Map<String, serde_json::Value>) {
    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
        let mut line: Vec<u8> = pending.drain(..=pos).collect();
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data.trim() == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(u) = value.get("usage") {
            if let Some(obj) = u.as_object() {
                usage.extend(obj.clone());
            }
        }
    }
}

/// Concatenated SSE `data:` payloads, used by tests to replay a stream.
#[cfg(test)]
pub(crate) fn collect_sse_data(raw: &[u8]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut pending = raw.to_vec();
    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
        let mut line: Vec<u8> = pending.drain(..=pos).collect();
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let Ok(line) = std::str::from_utf8(&line) else { continue };
        if let Some(data) = line.strip_prefix("data: ") {
            if data.trim() != "[DONE]" {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    out.push(v);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Application wiring (port of main.py)
// ---------------------------------------------------------------------------

/// The router served by [`serve`] — every route from `routes/*` plus CORS/trace
/// middleware (Python's FastAPI exception handlers map to explicit error
/// responses inside each handler).
pub fn app(db_path: PathBuf) -> Router {
    Router::new()
        .merge(system::router())
        .merge(models::router())
        .merge(mcp::router())
        .merge(messages::router())
        .merge(chat::router())
        .merge(crate::compression::router::router(db_path.clone()))
        .layer(
            tower_http::trace::TraceLayer::new_for_http().make_span_with(
                tower_http::trace::DefaultMakeSpan::new().include_headers(false),
            ),
        )
        .layer(
            tower_http::cors::CorsLayer::permissive()
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::OPTIONS,
                ])
                .max_age(Duration::from_secs(600)),
        )
}

/// Port of the FastAPI `lifespan` startup: warm the config, register the
/// Hyperspace model list for routing, and install the model-mapper hook.
pub async fn startup() {
    imc_proxy_types::get_config();
    crate::hyperspace::install_model_mapping_hook();
    match crate::hyperspace::fetch_hyperspace_models().await {
        Ok(data) => imc_proxy_types::logger::create_handler_logger("main").info(&format!(
            "[Hyperspace] Startup model cache warmed with {} models",
            data.len()
        )),
        Err(e) => imc_proxy_types::logger::create_handler_logger("main")
            .warning(&format!("[Hyperspace] Startup model cache failed: {e:#}")),
    }
}

/// Bind 127.0.0.1:{port} and serve until Ctrl-C.
pub fn serve(port: u16) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let addr = format!("127.0.0.1:{port}");
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            let log = imc_proxy_types::logger::create_handler_logger("main");
            log.info(&format!("Starting up proxy application on {addr}..."));
            startup().await;
            // Both the request-path compressor and the /v1/retrieve endpoint
            // must open the same file, or a handle written during compression
            // cannot be resolved afterwards.
            let ccr_db = crate::compression::context::ccr_db_path();
            let app = app(ccr_db);
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown_signal())
                .await?;
            log.info("Shutting down proxy application...");
            imc_proxy_types::logger::shutdown_logger_runtime();
            Ok(())
        })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boot the router on an ephemeral port so the wiring itself is asserted
    /// (no extra test-only dependencies).
    async fn serve_app() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let db_path = std::path::PathBuf::from("/tmp/test_ccr.db");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app(db_path)).await;
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn router_serves_health_and_404s_unknown_paths() {
        let (base, handle) = serve_app().await;
        let client = reqwest::Client::builder().build().unwrap();

        let health = client.get(format!("{base}/health")).send().await.unwrap();
        assert_eq!(health.status().as_u16(), 200);
        assert_eq!(
            health.json::<serde_json::Value>().await.unwrap(),
            serde_json::json!({"status": "ok"})
        );

        let missing = client.get(format!("{base}/nope")).send().await.unwrap();
        assert_eq!(missing.status().as_u16(), 404);

        // GET on a POST-only route is rejected, proving the method is bound.
        let wrong_method = client.get(format!("{base}/v1/messages")).send().await.unwrap();
        assert_eq!(wrong_method.status().as_u16(), 405);

        handle.abort();
    }

    #[tokio::test]
    async fn messages_route_rejects_invalid_payload_without_upstream_call() {
        let (base, handle) = serve_app().await;
        let client = reqwest::Client::builder().build().unwrap();
        let resp = client
            .post(format!("{base}/v1/messages"))
            .header("content-type", "application/json")
            .body("not json".to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 422);
        handle.abort();
    }

    #[tokio::test]
    async fn error_response_matches_forward_generic_error_shape() {
        let resp = error_response(500, "kaboom");
        assert_eq!(resp.status(), 500);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8_lossy(&bytes).to_string();
        assert!(body.contains("\"message\":\"kaboom\""), "{body}");
        assert!(body.contains("\"type\":\"error\""));
    }

    #[test]
    fn harvest_consumes_complete_lines_only() {
        let mut pending = b"data: {\"usage\":{\"input_tokens\":7}}\n\nda".to_vec();
        let mut usage = serde_json::Map::new();
        harvest_sse_usage(&mut pending, &mut usage);
        assert_eq!(usage.get("input_tokens"), Some(&serde_json::json!(7)));
        assert_eq!(pending, b"da".to_vec(), "partial line is preserved");

        // A second flush completes it.
        pending.extend_from_slice(b"ta: {\"usage\":{\"output_tokens\":3}}\n");
        harvest_sse_usage(&mut pending, &mut usage);
        assert_eq!(usage.get("output_tokens"), Some(&serde_json::json!(3)));
        assert!(pending.is_empty());
    }

    #[test]
    fn harvest_ignores_done_marker_and_bad_json() {
        let mut pending =
            b"data: [DONE]\ndata: {broken}\ndata: {\"usage\":{\"output_tokens\":1}}\n".to_vec();
        let mut usage = serde_json::Map::new();
        harvest_sse_usage(&mut pending, &mut usage);
        assert_eq!(usage.len(), 1);
        assert_eq!(usage.get("output_tokens"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn collect_sse_data_parses_frames() {
        let values = collect_sse_data(b"event: message_start\ndata: {\"a\":1}\n\ndata: [DONE]\n");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], serde_json::json!({"a": 1}));
    }

    #[test]
    fn relay_helpers_are_reachable() {
        let mut headers = HeaderMap::new();
        headers.insert("content-length", "5".parse().unwrap());
        let stripped = crate::providers::provider_proxy::strip_response_headers(headers);
        assert!(stripped.get("content-length").is_none());
    }
}