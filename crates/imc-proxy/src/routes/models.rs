//! Port of `main/proxy/routes/models.py` — `/v1/models` and
//! `/v1/hyperspace-models`.

use std::sync::{Mutex, OnceLock};

use axum::routing::get;
use axum::Router;
use imc_proxy_copilot::chat_completions::ensure_copilot_token;
use imc_proxy_types::api_config::{copilot_base_url, github_copilot_headers};
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::state::state;
use serde_json::{json, Value};

use crate::github::token_provider;
use crate::hyperspace::{fetch_hyperspace_models, registered_models};
use crate::routes::system::json_response;

/// Python's module-level `_models_cache`.
fn models_cache() -> &'static Mutex<Option<Value>> {
    static CACHE: OnceLock<Mutex<Option<Value>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Python's `_hyperspace_model_data`.
fn hyperspace_model_data() -> &'static Mutex<Vec<Value>> {
    static DATA: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
    DATA.get_or_init(|| Mutex::new(Vec::new()))
}

/// Port of `services/copilot/get_models.py`.
pub async fn get_models() -> Result<Value, String> {
    ensure_copilot_token(&token_provider()).await.map_err(|e| format!("{e:#}"))?;

    let st = state();
    let url = format!("{}/models", copilot_base_url(&st));
    let headers = github_copilot_headers(&st, None, false);

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .headers(super::header_map_from_btree(&headers))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if status != 200 {
        create_handler_logger("main")
            .error(&format!("Failed to fetch models: {status} - {text}"));
        return Ok(json!({"data": []}));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Port of `fetch_models` — cached list of supported Copilot models.
pub async fn fetch_models() -> axum::response::Response {
    if let Some(cached) = models_cache().lock().ok().and_then(|c| c.clone()) {
        return json_response(&cached);
    }
    match get_models().await {
        Ok(value) => {
            if let Ok(mut c) = models_cache().lock() {
                *c = Some(value.clone());
            }
            json_response(&value)
        }
        Err(e) => {
            create_handler_logger("main")
                .warning(&format!("[Models] Failed to fetch Copilot models: {e}"));
            json_response(&json!({"object": "list", "data": [], "error": e}))
        }
    }
}

/// Port of `fetch_hyperspace_models` — cached, and registers the ids for routing.
pub async fn fetch_hyperspace_models_route() -> axum::response::Response {
    if let Ok(data) = hyperspace_model_data().lock() {
        if !data.is_empty() {
            let payload = json!({"data": data.clone(), "cached": true});
            drop(data);
            return json_response(&payload);
        }
    }
    match fetch_hyperspace_models().await {
        Ok(data) => {
            if let Ok(mut c) = hyperspace_model_data().lock() {
                *c = data.clone();
            }
            json_response(&json!({"data": data}))
        }
        Err(e) => json_response(&json!({"data": [], "error": format!("{e:#}")})),
    }
}

/// Model ids currently registered for Hyperspace routing.
pub async fn hyperspace_model_ids() -> axum::response::Response {
    json_response(&json!({"data": registered_models()}))
}

/// Routes served by this module.
pub fn router() -> Router {
    Router::new()
        .route("/v1/models", get(fetch_models))
        .route("/v1/hyperspace-models", get(fetch_hyperspace_models_route))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_starts_empty_and_roundtrips() {
        // Single critical section: `Mutex` is not re-entrant.
        let mut c = models_cache().lock().unwrap();
        assert!(c.is_none());
        *c = Some(json!({"data": []}));
        assert!(c.is_some());
        *c = None;
    }

    #[test]
    fn hyperspace_data_cache_starts_empty() {
        assert!(hyperspace_model_data().lock().unwrap().is_empty());
    }

    #[test]
    fn header_helper_keeps_authorization() {
        let headers = super::super::header_map_from_btree(&imc_proxy_types::api_config::github_copilot_headers(
            &state(), None, false,
        ));
        assert!(headers.contains_key("authorization"));
        assert!(headers.contains_key("editor-version"));
    }
}