//! Embedded static files for the web dashboard.
//!
//! The Next.js dashboard is built with `output: 'export'` into `web-dashboard/out`,
//! and embedded into the binary at compile time via `rust-embed`.
//! This allows `imc` to serve the web dashboard on port 12345 (and 4789)
//! with ZERO external dependencies (no Node.js, no npm, no python).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web-dashboard/out"]
pub struct WebAssets;

/// Locate static asset either embedded in binary or directly from disk checkout.
pub fn get_asset(path: &str) -> Option<Cow<'static, [u8]>> {
    // 1. Try embedded asset
    if let Some(file) = WebAssets::get(path) {
        return Some(file.data);
    }

    // 2. Fallback to filesystem
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("IMC_WEB_DIR") {
        if !dir.is_empty() {
            candidates.push(PathBuf::from(dir).join("out"));
        }
    }
    candidates.push(PathBuf::from("web-dashboard/out"));
    candidates.push(PathBuf::from("/root/imc-rs/web-dashboard/out"));
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe.parent().map(Path::to_path_buf);
        for _ in 0..6 {
            if let Some(p) = cur {
                candidates.push(p.join("web-dashboard/out"));
                cur = p.parent().map(Path::to_path_buf);
            } else {
                break;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/imc/web-dashboard/out"));
    }

    for dir in candidates {
        let file_path = dir.join(path);
        if file_path.is_file() {
            if let Ok(bytes) = std::fs::read(&file_path) {
                return Some(Cow::Owned(bytes));
            }
        }
    }

    None
}

/// Serve embedded static files or fallback to index.html for SPA routes.
pub async fn static_handler(uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html".to_string()
    } else {
        raw_path.to_string()
    };

    // 1. Exact match (assets, _next, favicon.ico, etc.)
    if let Some(data) = get_asset(&path) {
        return serve_file(&path, data);
    }

    // 2. Try adding .html (e.g. /search -> search.html)
    let html_path = format!("{}.html", path.trim_end_matches('/'));
    if let Some(data) = get_asset(&html_path) {
        return serve_file(&html_path, data);
    }

    // 3. Try directory index (e.g. /projects/detail -> projects/detail/index.html)
    let dir_index = format!("{}/index.html", path.trim_end_matches('/'));
    if let Some(data) = get_asset(&dir_index) {
        return serve_file(&dir_index, data);
    }

    // 4. Return 404 with 404.html if available
    if let Some(data) = get_asset("404.html").or_else(|| get_asset("_not-found.html")) {
        let mut resp = serve_file("404.html", data);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_embedded_assets() {
        let files: Vec<String> = WebAssets::iter().map(|s| s.to_string()).collect();
        println!("Embedded count: {}, files: {:?}", files.len(), &files[..files.len().min(10)]);
        assert!(WebAssets::get("index.html").is_some(), "index.html missing from WebAssets");
        assert!(WebAssets::get("404.html").is_some(), "404.html missing from WebAssets");
    }
}
