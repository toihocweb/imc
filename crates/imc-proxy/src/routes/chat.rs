//! Port of `main/proxy/routes/chat.py` — `/v1/chat/completions`.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use imc_proxy_copilot::chat_completions::chat_completions;
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::model_mapping::{apply_model_mapping, get_mapped_provider};
use imc_proxy_types::types::chat_completions::ChatCompletionsPayload;

use super::{error_response, relay};
use crate::github::token_provider;
use crate::providers::provider_proxy::{
    build_provider_upstream_headers, forward_provider_chat_completions, resolve_provider,
};

fn downcased(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            out.insert(name.as_str().to_string(), v.to_string());
        }
    }
    out
}

/// Port of `proxy_chat_completions`.
pub async fn proxy_chat_completions(
    State(_state): State<()>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let log = create_handler_logger("main");
    let mut payload: ChatCompletionsPayload = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => {
            return error_response(400, &format!("invalid chat completions payload: {e}"));
        }
    };
    payload.model = apply_model_mapping(&payload.model);
    let model = payload.model.clone();
    let wants_stream = payload.stream.unwrap_or(false);

    let model_lower = model.to_lowercase();
    let is_reasoning_model = model_lower.contains("sol")
        || model_lower.contains("luna")
        || model_lower.contains("astra")
        || model_lower.contains("terra")
        || model_lower.contains("gpt-6")
        || model_lower.contains("gpt-5")
        || model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.starts_with("o4")
        || model_lower.contains("reasoning")
        || model_lower.contains("thinking");

    if is_reasoning_model && (payload.reasoning_effort.is_none() || payload.reasoning_effort.as_deref() == Some("none")) {
        payload.reasoning_effort = Some("high".to_string());
    }

    // A model mapped to `provider:<id>` with that provider configured is a
    // straight passthrough to that upstream.
    if let Some(provider_id) = get_mapped_provider(&model) {
        if let Some(cfg) = resolve_provider(&provider_id) {
            let upstream_headers = build_provider_upstream_headers(&cfg, &downcased(&headers));
            match forward_provider_chat_completions(&cfg, &payload, &upstream_headers).await {
                Ok(resp) => {
                    return relay(resp, model, wants_stream, "/v1/chat/completions").await;
                }
                Err(e) => return error_response(502, &format!("{e:#}")),
            }
        }
    }

    let upstream = match chat_completions(&token_provider(), &payload).await {
        Ok(r) => r,
        Err(e) => {
            log.warning(&format!("[Proxy] chat completions failed ({e:#}), trying Hyperspace fallback..."));
            let hcfg = crate::hyperspace::load_hyperspace_config();
            if !hcfg.base_url.is_empty() {
                let url = format!("{}/chat/completions", hcfg.base_url);
                if let Ok(client) = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build() {
                    let mut req = client.post(&url).header("content-type", "application/json");
                    if !hcfg.api_key.is_empty() {
                        req = req.header("authorization", format!("Bearer {}", hcfg.api_key));
                    }
                    let mut hp_payload = payload.clone();
                    hp_payload.model = crate::hyperspace::resolve_hyperspace_model(&hp_payload.model);
                    if let Ok(mut body) = serde_json::to_value(&hp_payload) {
                        imc_proxy_copilot::chat_completions::sanitize_message_content(&mut body);
                        if let Ok(resp) = req.json(&body).send().await {
                            return relay(resp, model, wants_stream, "/v1/chat/completions").await;
                        }
                    }
                }
            }
            return error_response(503, &format!("Copilot is optional and not configured: {e:#}"));
        }
    };

    relay(upstream, model, wants_stream, "/v1/chat/completions").await
}

/// Routes served by this module.
pub fn router() -> Router {
    Router::new().route("/v1/chat/completions", post(proxy_chat_completions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_json_with_400() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let resp = rt.block_on(proxy_chat_completions(
            State(()),
            HeaderMap::new(),
            "not json".to_string(),
        ));
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn model_mapping_is_applied_before_routing() {
        // `apply_model_mapping` is the same call Python makes first thing in
        // the handler; a known-prefix model with no configured mapping passes
        // through unchanged.
        assert_eq!(apply_model_mapping("claude-test-xyz-000"), "claude-test-xyz-000");
        assert_eq!(apply_model_mapping("gpt-test-xyz-000"), "gpt-test-xyz-000");
        assert!(get_mapped_provider("claude-test-xyz-000").is_none());
    }
}