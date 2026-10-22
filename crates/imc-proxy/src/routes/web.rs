//! Embedded static files for the web dashboard.
//!
//! The Next.js dashboard is built with `output: 'export'` into `web-dashboard/out`,
//! and embedded into the binary at compile time via `rust-embed`.
//! This allows `imc` to serve the web dashboard on port 12345 (and 4789)
//! with ZERO external dependencies (no Node.js, no npm, no python).

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web-dashboard/out"]
pub struct WebAssets;

/// Serve embedded static files or fallback to index.html for SPA routes.
pub async fn static_handler(uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html".to_string()
    } else {
        raw_path.to_string()
    };

    // 1. Exact match (assets, _next, favicon.ico, etc.)
    if let Some(file) = WebAssets::get(&path) {
        return serve_file(&path, file.data);
    }

    // 2. Try adding .html (e.g. /search -> search.html)
    let html_path = format!("{}.html", path.trim_end_matches('/'));
    if let Some(file) = WebAssets::get(&html_path) {
        return serve_file(&html_path, file.data);
    }

    // 3. Try directory index (e.g. /projects/detail -> projects/detail/index.html)
    let dir_index = format!("{}/index.html", path.trim_end_matches('/'));
    if let Some(file) = WebAssets::get(&dir_index) {
        return serve_file(&dir_index, file.data);
    }

    // 4. Return 404 with 404.html if available
    if let Some(file) = WebAssets::get("404.html").or_else(|| WebAssets::get("_not-found.html")) {
        let mut resp = serve_file("404.html", file.data);
        *resp.status_mut() = StatusCode::NOT_FOUND;
        return resp;
    }

    (StatusCode::NOT_FOUND, "Web dashboard asset not found").into_response()
}

fn serve_file(path: &str, data: std::borrow::Cow<'static, [u8]>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let content_type = HeaderValue::from_str(mime.as_ref())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(data))
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "response build error").into_response())
}
