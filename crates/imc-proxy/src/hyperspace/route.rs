//! Port of `main/proxy/services/hyperspace/route.py` — forwards
//! `/v1/messages` to the OpenAI-compatible IMC endpoint (`llm.base_url`,
//! e.g. `http://localhost:6655/litellm/v1`).
//!
//! The Anthropic payload is translated to OpenAI chat completions and streamed
//! back; the caller (routes/messages) re-translates the SSE frames into
//! Anthropic events.

use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use imc_proxy_copilot::chat_completions::sanitize_message_content;
use imc_proxy_copilot::non_stream_translation::translate_to_openai;
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::types::anthropic::AnthropicMessagesPayload;
use serde_json::Value;

/// Port of the `_hyperspace_models` module-global set.
fn model_registry() -> &'static RwLock<HashSet<String>> {
    static REGISTRY: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Port of `load_hyperspace_config` — `llm.base_url` / `llm.api_key` from
/// `~/.indexer_config.yaml` (read through `imc-core`'s settings loader, which
/// also accepts the renamed `~/.imc_config.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HyperspaceConfig {
    pub base_url: String,
    pub api_key: String,
}

pub fn load_hyperspace_config() -> HyperspaceConfig {
    let settings = imc_core::settings::load();
    let Some(llm) = settings.llm else {
        return HyperspaceConfig::default();
    };
    HyperspaceConfig {
        base_url: llm
            .base_url
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string(),
        api_key: llm.api_key.unwrap_or_default(),
    }
}

/// Port of `register_hyperspace_models` — called by the models route and by the
/// startup warm-up so routing knows the available model ids.
pub fn register_hyperspace_models(model_ids: &[String]) {
    if let Ok(mut registry) = model_registry().write() {
        *registry = model_ids.iter().cloned().collect();
    }
}

/// Port of `is_hyperspace_model`.
pub fn is_hyperspace_model(model_id: &str) -> bool {
    model_registry()
        .read()
        .map(|r| r.contains(model_id))
        .unwrap_or(false)
}

/// Model ids known to Hyperspace (used by the models route + tests).
pub fn registered_models() -> Vec<String> {
    let mut ids: Vec<String> = model_registry()
        .read()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// Wire `imc-proxy-types`' model mapper to this crate's registry. Called once at
/// server start so `apply_model_mapping` treats unknown-but-Hyperspace models
/// as known.
pub fn install_model_mapping_hook() {
    imc_proxy_types::model_mapping::set_hyperspace_model_check(is_hyperspace_model);
}

/// Adjusts reasoning_effort for function tools on /chat/completions.
/// OpenAI / Azure endpoints reject function tools when reasoning_effort is active.
/// If the model is a reasoning model and tools are present, set reasoning_effort to "none".
pub fn adjust_reasoning_effort_for_tools(body: &mut Value, model: &str) {
    let has_tools = body
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    if !has_tools {
        return;
    }

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

    if is_reasoning_model && body.get("reasoning_effort").is_none() {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("reasoning_effort".to_string(), serde_json::json!("none"));
        }
    }
}

/// Port of `call_hyperspace`.
///
/// Returns the raw upstream response **unread** so the caller can forward SSE
/// chunks as they arrive.
pub async fn call_hyperspace(
    payload: &AnthropicMessagesPayload,
    _anthropic_beta: Option<&str>,
) -> Result<reqwest::Response> {
    let cfg = load_hyperspace_config();
    if cfg.base_url.is_empty() {
        anyhow::bail!("Hyperspace base_url not configured in ~/.indexer_config.yaml");
    }

    let openai_payload = translate_to_openai(payload);
    let mut openai_data =
        serde_json::to_value(&openai_payload).context("serialize openai payload")?;

    // Guard: OpenAI-compatible endpoints reject `content: null` unless the
    // message is an assistant turn carrying tool calls.
    sanitize_message_content(&mut openai_data);
    adjust_reasoning_effort_for_tools(&mut openai_data, &payload.model);

    let url = format!("{}/chat/completions", cfg.base_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("build http client")?;

    let request = client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", cfg.api_key))
        .json(&openai_data)
        .build()
        .context("build hyperspace request")?;

    let response = client.execute(request).await.context("post hyperspace")?;
    Ok(response)
}

/// Port of the startup warm-up in `main.py` (and the body of
/// `GET /v1/hyperspace-models`): fetch the model list and register it.
///
/// Returns the raw `data` array on success.
pub async fn fetch_hyperspace_models() -> Result<Vec<serde_json::Value>> {
    let log = create_handler_logger("main");
    let cfg = load_hyperspace_config();
    if cfg.base_url.is_empty() {
        anyhow::bail!("base_url not configured");
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .get(format!("{}/models", cfg.base_url))
        .header("authorization", format!("Bearer {}", cfg.api_key))
        .send()
        .await
        .context("fetch hyperspace models")?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if status != 200 {
        log.error(&format!("[Hyperspace] models fetch failed: HTTP {status}"));
        anyhow::bail!("HTTP {status}");
    }
    let data: Vec<serde_json::Value> = serde_json::from_str(&text)
        .ok()
        .and_then(|v: serde_json::Value| {
            v.get("data")
                .and_then(|d| d.as_array())
                .cloned()
        })
        .unwrap_or_default();
    let ids: Vec<String> = data
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    register_hyperspace_models(&ids);
    log.info(&format!(
        "[Hyperspace] Cached {} models for routing",
        ids.len()
    ));
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roundtrip() {
        let before = registered_models();
        register_hyperspace_models(&["gemini-3.1-flash-lite".to_string()]);
        assert!(is_hyperspace_model("gemini-3.1-flash-lite"));
        assert!(!is_hyperspace_model("claude-sonnet-4"));
        register_hyperspace_models(&before);
    }

    #[test]
    fn unconfigured_base_url_is_trailing_slash_free() {
        let cfg = load_hyperspace_config();
        assert!(!cfg.base_url.ends_with('/'));
    }

    #[tokio::test]
    async fn call_hyperspace_without_base_url_fails() {
        let cfg = load_hyperspace_config();
        if cfg.base_url.is_empty() {
            let payload = AnthropicMessagesPayload::default();
            let err = call_hyperspace(&payload, None).await.unwrap_err();
            assert!(err.to_string().contains("base_url not configured"));
        }
    }

    #[test]
    fn adjust_reasoning_effort_for_tools_sets_none_when_tools_present_on_reasoning_model() {
        let mut body = serde_json::json!({
            "model": "gpt-6.1-sol",
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{"type": "function", "function": {"name": "test"}}]
        });
        adjust_reasoning_effort_for_tools(&mut body, "gpt-6.1-sol");
        assert_eq!(body.get("reasoning_effort").and_then(|v| v.as_str()), Some("none"));
    }

    #[test]
    fn adjust_reasoning_effort_for_tools_leaves_alone_without_tools() {
        let mut body = serde_json::json!({
            "model": "gpt-6.1-sol",
            "messages": [{"role": "user", "content": "hello"}]
        });
        adjust_reasoning_effort_for_tools(&mut body, "gpt-6.1-sol");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn adjust_reasoning_effort_for_tools_leaves_non_reasoning_model_alone() {
        let mut body = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "hello"}],
            "tools": [{"type": "function", "function": {"name": "test"}}]
        });
        adjust_reasoning_effort_for_tools(&mut body, "claude-3-5-sonnet");
        assert!(body.get("reasoning_effort").is_none());
    }
}