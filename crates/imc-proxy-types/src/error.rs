//! Port of `main/proxy/lib/error.py`.
//!
//! `fastapi.HTTPException` / `starlette.JSONResponse` have no Rust equivalent
//! here, so [`HTTPError`] is a plain error type and [`JsonResponse`] is the
//! (status, body, headers) triple the API layer hands back to the router.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::logger::create_handler_logger;

/// Port of the `HTTPError` exception class.
#[derive(Debug, Clone, Default)]
pub struct HTTPError {
    pub message: String,
    pub status_code: u16,
    pub response_text: String,
    pub headers: BTreeMap<String, String>,
}

impl HTTPError {
    pub fn new(
        message: impl Into<String>,
        status_code: u16,
        response_text: impl Into<String>,
        headers: Option<BTreeMap<String, String>>,
    ) -> Self {
        Self {
            message: message.into(),
            status_code,
            response_text: response_text.into(),
            headers: headers.unwrap_or_default(),
        }
    }
}

impl std::fmt::Display for HTTPError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for HTTPError {}

/// Stand-in for `starlette.responses.JSONResponse`.
#[derive(Debug, Clone)]
pub struct JsonResponse {
    pub status_code: u16,
    pub content: Value,
    pub headers: BTreeMap<String, String>,
}

impl JsonResponse {
    /// Serialized body, the way FastAPI would send it.
    pub fn body(&self) -> String {
        serde_json::to_string(&self.content).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Port of `error.forward_error`.
pub fn forward_error(error: &HTTPError) -> JsonResponse {
    let log = create_handler_logger("error");
    if error.status_code == 429 {
        // Forward rate-limit headers only: `retry-after` and any `x-*`.
        let mut headers_to_forward = BTreeMap::new();
        for (k, v) in &error.headers {
            let lower_k = k.to_lowercase();
            if lower_k == "retry-after" || lower_k.starts_with("x-") {
                headers_to_forward.insert(k.clone(), v.clone());
            }
        }
        log.error(&format!("HTTP error: {}", error.response_text));
        JsonResponse {
            status_code: error.status_code,
            content: json!({
                "error": { "message": error.response_text, "type": "error" }
            }),
            headers: headers_to_forward,
        }
    } else {
        log.error(&format!("HTTP error: {}", error.response_text));
        JsonResponse {
            status_code: error.status_code,
            content: json!({
                "error": { "message": error.response_text, "type": "error" }
            }),
            headers: BTreeMap::new(),
        }
    }
}

/// Port of the non-`HTTPError` branch: any other exception becomes a 500.
pub fn forward_generic_error(message: &str) -> JsonResponse {
    create_handler_logger("error").error(&format!("Error occurred: {message}"));
    JsonResponse {
        status_code: 500,
        content: json!({ "error": { "message": message, "type": "error" } }),
        headers: BTreeMap::new(),
    }
}