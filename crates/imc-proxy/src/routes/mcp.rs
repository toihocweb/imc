//! MCP over HTTP (`POST /mcp`), the streamable-HTTP transport.
//!
//! The same server answers on stdio (`imc mcp`) and here, so an MCP client
//! can point at `http://127.0.0.1:12345/mcp` instead of spawning a process.
//!
//! The project the tools work on is taken from the `x-imc-root` header, and
//! falls back to the globally active project.

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use serde_json::{json, Value};

/// The header a client can use to pick the project.
const ROOT_HEADER: &str = "x-imc-root";

pub fn router() -> Router {
    Router::new()
        .route("/mcp", post(post_mcp).get(get_mcp).delete(delete_mcp))
        .route("/mcp/", post(post_mcp))
}

/// The project the tools run against.
fn resolve_root(headers: &HeaderMap) -> PathBuf {
    if let Some(root) = headers
        .get(ROOT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return PathBuf::from(root);
    }
    // The active project, else the working directory.
    let cfg = imc_core::config::Config::default();
    let active = std::fs::read_to_string(cfg.data_dir.join("active_project.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    active.unwrap_or_else(|| PathBuf::from("."))
}

/// `POST /mcp` — one JSON-RPC request (or a batch).
async fn post_mcp(headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let root = resolve_root(&headers);
    let Ok(parsed) = serde_json::from_slice::<Value>(&body) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": -32700, "message": "Parse error"}}),
        );
    };

    // A batch is an array of requests.
    if let Some(items) = parsed.as_array() {
        let mut out = Vec::new();
        for item in items {
            if let Some(resp) = imc_mcp::handle_jsonrpc(&root, item) {
                out.push(resp);
            }
        }
        if out.is_empty() {
            // Every message was a notification: no body, 202.
            return Response::builder()
                .status(StatusCode::ACCEPTED)
                .body(Body::empty())
                .expect("accepted");
        }
        return json_response(StatusCode::OK, Value::Array(out));
    }

    match imc_mcp::handle_jsonrpc(&root, &parsed) {
        Some(resp) => json_response(StatusCode::OK, resp),
        // A notification gets no reply.
        None => Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Body::empty())
            .expect("accepted"),
    }
}

/// `GET /mcp` — the server→client stream. The server pushes nothing, so this
/// is a keep-alive that ends when the client disconnects.
async fn get_mcp() -> Response {
    let stream = async_stream::stream! {
        // A comment frame keeps proxies from closing the connection.
        yield Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b": imc mcp ready\n\n"));
    };
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(stream))
        .expect("sse response")
}

/// `DELETE /mcp` — end the session.
async fn delete_mcp() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .expect("no content")
}

fn json_response(status: StatusCode, value: Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .expect("json response")
}
