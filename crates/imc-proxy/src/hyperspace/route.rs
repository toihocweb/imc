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
    let lower = model_id.to_lowercase();
    if lower.contains("gemini") {
        return true;
    }
    let suffix = format!("/{lower}");
    model_registry()
        .read()
        .map(|r| {
            r.contains(model_id)
                || r.iter().any(|m| {
                    let m_lower = m.to_lowercase();
                    m_lower == lower || m_lower.ends_with(&suffix)
                })
        })
        .unwrap_or(false)
}

/// Resolves a requested model name against the Hyperspace registry.
/// Supports bare model names (e.g. "gemini-3.8-flash-high" -> "cuzem3/gemini-3.8-flash-high")
/// or aliases (e.g. "gemini", "gemini-flash" -> "gemini-3.8-flash").
pub fn resolve_hyperspace_model(model: &str) -> String {
    let reg = model_registry().read().ok();
    let Some(reg) = reg else {
        return model.to_string();
    };

    if reg.contains(model) {
        return model.to_string();
    }

    let lower = model.to_lowercase();
    let suffix = format!("/{lower}");

    // Look for exact suffix match (e.g. ".../gemini-3.8-flash-high")
    if let Some(matched) = reg.iter().find(|m| m.to_lowercase().ends_with(&suffix)) {
        return matched.clone();
    }

    // Common aliases for Gemini
    if lower == "gemini" || lower == "gemini-flash" {
        if let Some(m) = reg.iter().find(|m| *m == "gemini-3.8-flash") {
            return m.clone();
        }
        if let Some(m) = reg.iter().find(|m| m.ends_with("/gemini-3.8-flash")) {
            return m.clone();
        }
        if let Some(m) = reg.iter().find(|m| m.to_lowercase().contains("gemini") && m.to_lowercase().contains("flash")) {
            return m.clone();
        }
    }

    if lower != model.to_lowercase() {
        return lower;
    }

    model.to_string()
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

    let resolved_model = resolve_hyperspace_model(&payload.model);
    let mut payload = payload.clone();
    payload.model = resolved_model;

    let openai_payload = translate_to_openai(&payload);
    let mut openai_data =
        serde_json::to_value(&openai_payload).context("serialize openai payload")?;

    // Guard: OpenAI-compatible endpoints reject `content: null` unless the
    // message is an assistant turn carrying tool calls.
    sanitize_message_content(&mut openai_data);

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
    fn gemini_models_are_treated_as_hyperspace_and_resolved() {
        let before = registered_models();
        register_hyperspace_models(&[
            "gemini-3.8-flash".to_string(),
            "cuzem3/gemini-3.8-flash-high".to_string(),
        ]);

        assert!(is_hyperspace_model("gemini"));
        assert!(is_hyperspace_model("gemini-3.8-flash"));
        assert!(is_hyperspace_model("gemini-3.8-flash-high"));

        assert_eq!(resolve_hyperspace_model("gemini-3.8-flash"), "gemini-3.8-flash");
        assert_eq!(resolve_hyperspace_model("gemini-3.8-flash-high"), "cuzem3/gemini-3.8-flash-high");
        assert_eq!(resolve_hyperspace_model("gemini"), "gemini-3.8-flash");

        register_hyperspace_models(&before);
    }
}