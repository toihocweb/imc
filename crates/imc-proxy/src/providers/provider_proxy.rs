//! Port of `main/proxy/services/providers/provider_proxy.py`.
//!
//! Two responsibilities:
//!
//! 1. **Routing decision** — which upstream a `/v1/messages` request goes to:
//!    a configured provider passthrough, Hyperspace, Copilot's Responses API,
//!    or Copilot's native Anthropic endpoint. [`decide_route`] is pure and
//!    unit-tested.
//! 2. **Forwarding** — build the upstream headers, POST the body and return the
//!    response **unread** so the route can stream SSE without buffering.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use imc_proxy_types::config_store::{get_config, ProviderConfig};
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::types::anthropic::AnthropicMessagesPayload;
use imc_proxy_types::types::chat_completions::ChatCompletionsPayload;
use imc_proxy_types::types::responses::ResponsesPayload;

/// Headers forwarded for every provider.
pub const SHARED_FORWARDABLE_HEADERS: &[&str] = &["accept", "user-agent"];
/// Extra headers only Anthropic-typed providers receive.
pub const ANTHROPIC_FORWARDABLE_HEADERS: &[&str] = &["anthropic-version", "anthropic-beta"];
/// Response hop-by-hop headers stripped before relaying upstream replies.
pub const STRIPPED_RESPONSE_HEADERS: &[&str] = &[
    "connection",
    "content-encoding",
    "content-length",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn lower_header_map(headers: &axum::http::HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            out.insert(name.as_str().to_string(), v.to_string());
        }
    }
    out
}

/// Port of `build_provider_upstream_headers`.
pub fn build_provider_upstream_headers(
    provider_config: &ProviderConfig,
    request_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let api_key = provider_config.api_key.clone().unwrap_or_default();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    headers.insert("accept".to_string(), "application/json".to_string());
    if provider_config.auth_type.as_deref() == Some("x-api-key") {
        headers.insert("x-api-key".to_string(), api_key);
    } else {
        headers.insert("authorization".to_string(), format!("Bearer {api_key}"));
    }

    for header_name in SHARED_FORWARDABLE_HEADERS {
        if let Some(value) = request_headers.get(*header_name) {
            headers.insert((*header_name).to_string(), value.clone());
        }
    }

    if provider_config.r#type.as_deref() == Some("anthropic") {
        for header_name in ANTHROPIC_FORWARDABLE_HEADERS {
            if let Some(value) = request_headers.get(*header_name) {
                headers.insert((*header_name).to_string(), value.clone());
            }
        }
    }

    headers
}

/// Port of `resolve_provider_request_url` — append the query string to the
/// provider's base url.
pub fn resolve_provider_request_url(base_url: &str, request_url_query: &str, path: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    if request_url_query.is_empty() {
        format!("{base_url}{path}")
    } else {
        format!("{base_url}{path}?{request_url_query}")
    }
}

/// Port of the header stripping applied to every relayed response.
pub fn strip_response_headers(mut headers: axum::http::HeaderMap) -> axum::http::HeaderMap {
    for name in STRIPPED_RESPONSE_HEADERS {
        if let Ok(h) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(h);
        }
    }
    headers
}

fn header_map_from_btree(headers: &BTreeMap<String, String>) -> axum::http::HeaderMap {
    let mut map = axum::http::HeaderMap::new();
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

// ---------------------------------------------------------------------------
// Routing decision
// ---------------------------------------------------------------------------

/// Where a `/v1/messages` request ends up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteTarget {
    /// Configured provider (`providers.<name>` in the config) — raw passthrough
    /// of the Anthropic body to `baseUrl + /v1/messages`.
    ProviderPassthrough(String),
    /// IMC's OpenAI-compatible endpoint (`llm.base_url`).
    Hyperspace,
    /// Copilot's OpenAI Responses endpoint (after translation).
    CopilotResponses,
    /// Copilot's native Anthropic endpoint (after translation + cache markers).
    CopilotMessages,
}

/// The full routing decision, kept separate from [`RouteTarget`] so the
/// individual flags can be asserted in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub target: RouteTarget,
    /// Provider prefix parsed from the model mapping (`"hyperspace:gpt-x"` -> `"hyperspace"`).
    pub explicit_provider: Option<String>,
    pub hyperspace: bool,
    pub responses: bool,
}

impl RouteDecision {
    pub fn is_passthrough(&self) -> bool {
        matches!(self.target, RouteTarget::ProviderPassthrough(_))
    }
}

/// Port of the route decision in `routes/messages.py`, plus the
/// passthrough-vs-upstream branch for `config.providers`.
///
/// Order:
/// 1. an explicit, enabled provider in `config.providers` -> passthrough
///    (unless it is `hyperspace`/`copilot`, which have dedicated branches),
/// 2. Hyperspace when the mapping says so, or when the model is registered
///    there / `anthropic--`-prefixed,
/// 3. the Responses API for `gpt-` / `o1` / `o3` model ids,
/// 4. otherwise Copilot's native Anthropic endpoint.
pub fn decide_route(
    explicit_provider: Option<&str>,
    mapped_model: &str,
    is_hyperspace: bool,
    providers: Option<&BTreeMap<String, ProviderConfig>>,
) -> RouteDecision {
    let explicit = explicit_provider.map(str::to_string);

    if let Some(name) = explicit_provider {
        if name != "hyperspace" && name != "copilot" {
            if let Some(cfg) = providers.and_then(|m| m.get(name)) {
                let enabled = cfg.enabled.unwrap_or(true);
                let has_base = cfg
                    .base_url
                    .as_deref()
                    .map(|b| !b.trim().is_empty())
                    .unwrap_or(false);
                if enabled && has_base {
                    return RouteDecision {
                        target: RouteTarget::ProviderPassthrough(name.to_string()),
                        explicit_provider: explicit,
                        hyperspace: false,
                        responses: false,
                    };
                }
            }
        }
    }

    let hyperspace = explicit_provider == Some("hyperspace")
        || (explicit_provider != Some("copilot")
            && (is_hyperspace || mapped_model.starts_with("anthropic--")));
    if hyperspace {
        return RouteDecision {
            target: RouteTarget::Hyperspace,
            explicit_provider: explicit,
            hyperspace: true,
            responses: false,
        };
    }

    let responses = mapped_model.starts_with("gpt-")
        || mapped_model.starts_with("o1")
        || mapped_model.starts_with("o3");
    RouteDecision {
        target: if responses {
            RouteTarget::CopilotResponses
        } else {
            RouteTarget::CopilotMessages
        },
        explicit_provider: explicit,
        hyperspace: false,
        responses,
    }
}

/// [`decide_route`] wired to the live config + Hyperspace registry.
pub fn route_for_model(
    _original_model: &str,
    mapped_model: &str,
    explicit_provider: Option<&str>,
) -> RouteDecision {
    let config = get_config();
    decide_route(
        explicit_provider,
        mapped_model,
        crate::hyperspace::is_hyperspace_model(mapped_model),
        config.providers.as_ref(),
    )
}

// ---------------------------------------------------------------------------
// Forwarding
// ---------------------------------------------------------------------------

fn client(timeout_secs: u64) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .context("build http client")
}

/// Shared body: `payload.model_dump(exclude_none=True)` equivalent.
fn dump<T: serde::Serialize>(payload: &T) -> Result<serde_json::Value> {
    serde_json::to_value(payload).context("serialize provider payload")
}

/// Port of `forward_provider_messages` — POST to `{baseUrl}/v1/messages`.
/// The response body is left unread so the caller can stream it.
pub async fn forward_provider_messages(
    provider_config: &ProviderConfig,
    payload: &AnthropicMessagesPayload,
    request_headers: &BTreeMap<String, String>,
) -> Result<reqwest::Response> {
    create_handler_logger("main").info(&format!("<-- model: {}", payload.model));
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, "", "/v1/messages");
    let headers = build_provider_upstream_headers(provider_config, request_headers);
    let body = dump(payload)?;
    client(60)?
        .post(&url)
        .headers(header_map_from_btree(&headers))
        .json(&body)
        .send()
        .await
        .context("post provider messages")
}

/// Port of `forward_provider_chat_completions`.
pub async fn forward_provider_chat_completions(
    provider_config: &ProviderConfig,
    payload: &ChatCompletionsPayload,
    request_headers: &BTreeMap<String, String>,
) -> Result<reqwest::Response> {
    create_handler_logger("main").info(&format!("<-- model: {}", payload.model));
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, "", "/v1/chat/completions");
    let headers = build_provider_upstream_headers(provider_config, request_headers);
    let mut body = dump(payload)?;
    crate::hyperspace::adjust_reasoning_effort_for_tools(&mut body, &payload.model);
    client(60)?
        .post(&url)
        .headers(header_map_from_btree(&headers))
        .json(&body)
        .send()
        .await
        .context("post provider chat completions")
}

/// Port of `forward_provider_responses` — Python uses a 5s connect timeout with
/// no read deadline so long streams are not cut off.
pub async fn forward_provider_responses(
    provider_config: &ProviderConfig,
    payload: &ResponsesPayload,
    request_headers: &BTreeMap<String, String>,
) -> Result<reqwest::Response> {
    create_handler_logger("main").info(&format!("<-- model: {}", payload.model));
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, "", "/v1/responses");
    let headers = build_provider_upstream_headers(provider_config, request_headers);
    let body = dump(payload)?;
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?
        .post(&url)
        .headers(header_map_from_btree(&headers))
        .json(&body)
        .send()
        .await
        .context("post provider responses")
}

/// Port of `forward_provider_models` (buffered — Python reads the body here).
pub async fn forward_provider_models(
    provider_config: &ProviderConfig,
    request_headers: &BTreeMap<String, String>,
) -> Result<reqwest::Response> {
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, "", "/v1/models");
    let headers = build_provider_upstream_headers(provider_config, request_headers);
    client(15)?
        .get(&url)
        .headers(header_map_from_btree(&headers))
        .send()
        .await
        .context("get provider models")
}

/// Port of `forward_provider_alpha_search`.
pub async fn forward_provider_alpha_search(
    provider_config: &ProviderConfig,
    request_headers: &BTreeMap<String, String>,
    query: &str,
    body: &[u8],
) -> Result<reqwest::Response> {
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, query, "/v1/alpha/search");
    let headers = build_provider_upstream_headers(provider_config, request_headers);
    client(60)?
        .post(&url)
        .headers(header_map_from_btree(&headers))
        .body(body.to_vec())
        .send()
        .await
        .context("post provider alpha search")
}

/// Port of `forward_provider_images` — `operation` is `generations` or `edits`.
/// Python forwards the incoming `content-type` (multipart for edits).
pub async fn forward_provider_images(
    provider_config: &ProviderConfig,
    request_headers: &BTreeMap<String, String>,
    content_type: Option<&str>,
    query: &str,
    body: Vec<u8>,
    operation: &str,
) -> Result<reqwest::Response> {
    let base = provider_config.base_url.clone().unwrap_or_default();
    let url = resolve_provider_request_url(&base, query, &format!("/v1/images/{operation}"));
    let mut headers = build_provider_upstream_headers(provider_config, request_headers);
    match content_type {
        Some(ct) => {
            headers.insert("content-type".to_string(), ct.to_string());
        }
        None if operation == "edits" => {
            headers.remove("content-type");
        }
        None => {}
    }
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15 * 60))
        .build()?
        .post(&url)
        .headers(header_map_from_btree(&headers))
        .body(body)
        .send()
        .await
        .context("post provider images")
}

/// Convenience for routes holding raw `axum` headers.
pub fn upstream_headers_from_axum(
    provider_config: &ProviderConfig,
    headers: &axum::http::HeaderMap,
) -> BTreeMap<String, String> {
    build_provider_upstream_headers(provider_config, &lower_header_map(headers))
}

/// Look up an enabled provider by id in the live config.
pub fn resolve_provider(provider_id: &str) -> Option<ProviderConfig> {
    get_config()
        .providers?
        .get(provider_id)
        .filter(|p| p.enabled.unwrap_or(true))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, base: &str, ptype: &str) -> (String, ProviderConfig) {
        (
            id.to_string(),
            ProviderConfig {
                r#type: Some(ptype.to_string()),
                enabled: Some(true),
                base_url: Some(base.to_string()),
                api_key: Some("sk-test".to_string()),
                auth_type: Some("authorization".to_string()),
                ..Default::default()
            },
        )
    }

    fn providers(list: Vec<(String, ProviderConfig)>) -> BTreeMap<String, ProviderConfig> {
        list.into_iter().collect()
    }

    // ── decide_route ─────────────────────────────────────────────────────────

    #[test]
    fn explicit_hyperspace_provider_routes_to_hyperspace() {
        let d = decide_route(Some("hyperspace"), "claude-sonnet-4", false, None);
        assert_eq!(d.target, RouteTarget::Hyperspace);
        assert!(d.hyperspace);
        assert!(!d.responses);
    }

    #[test]
    fn hyperspace_registry_and_prefix_route_to_hyperspace() {
        let d = decide_route(None, "gemini-3.1-flash-lite", true, None);
        assert_eq!(d.target, RouteTarget::Hyperspace);
        let d = decide_route(None, "anthropic--claude-3-5-sonnet", false, None);
        assert_eq!(d.target, RouteTarget::Hyperspace);
    }

    #[test]
    fn explicit_copilot_never_routes_to_hyperspace() {
        let d = decide_route(Some("copilot"), "gemini-3.1-flash-lite", true, None);
        assert_ne!(d.target, RouteTarget::Hyperspace);
        assert_eq!(d.target, RouteTarget::CopilotMessages, "unknown family -> native");
    }

    #[test]
    fn gpt_and_o_prefixes_use_the_responses_api() {
        for model in ["gpt-5.4", "gpt-5-mini", "o1-preview", "o3-mini"] {
            let d = decide_route(Some("copilot"), model, false, None);
            assert_eq!(d.target, RouteTarget::CopilotResponses, "{model}");
            assert!(d.responses);
        }
        let d = decide_route(None, "claude-sonnet-4", false, None);
        assert_eq!(d.target, RouteTarget::CopilotMessages);
        assert!(!d.responses);
    }

    #[test]
    fn configured_provider_wins_over_hyperspace_and_responses() {
        let (id, cfg) = provider("openrouter", "https://openrouter.ai/api", "openai");
        let map = providers(vec![(id.clone(), cfg)]);

        let d = decide_route(Some("openrouter"), "gpt-5.4", true, Some(&map));
        assert_eq!(d.target, RouteTarget::ProviderPassthrough("openrouter".into()));
        assert!(d.is_passthrough());
        assert!(!d.hyperspace);
        assert!(!d.responses);
    }

    #[test]
    fn disabled_or_baseless_provider_falls_through() {
        let (id, mut cfg) = provider("openrouter", "https://openrouter.ai/api", "openai");
        cfg.enabled = Some(false);
        let map = providers(vec![(id.clone(), cfg)]);
        let d = decide_route(Some("openrouter"), "gpt-5.4", false, Some(&map));
        assert_eq!(d.target, RouteTarget::CopilotResponses);

        let (id2, mut cfg2) = provider("blank", "", "openai");
        cfg2.base_url = Some("   ".into());
        let map = providers(vec![(id2, cfg2)]);
        let d = decide_route(Some("blank"), "claude-sonnet-4", false, Some(&map));
        assert_eq!(d.target, RouteTarget::CopilotMessages);
    }

    #[test]
    fn unknown_provider_prefix_without_config_falls_back() {
        let d = decide_route(Some("acme"), "gpt-5.4", false, None);
        assert_eq!(d.target, RouteTarget::CopilotResponses, "responses check ignores the prefix");
    }

    // ── headers / urls ───────────────────────────────────────────────────────

    #[test]
    fn bearer_auth_by_default_and_x_api_key_when_asked() {
        let (_, anthropic) = provider("anth", "https://api.anthropic.com", "anthropic");
        let mut req = BTreeMap::new();
        req.insert("accept".into(), "text/event-stream".into());
        req.insert("user-agent".into(), "claude-code".into());
        req.insert("anthropic-version".into(), "2023-06-01".into());
        req.insert("anthropic-beta".into(), "prompt-caching-2024-07-31".into());

        let h = build_provider_upstream_headers(&anthropic, &req);
        assert_eq!(h["authorization"], "Bearer sk-test");
        assert_eq!(h["content-type"], "application/json");
        assert_eq!(h["user-agent"], "claude-code");
        assert_eq!(h["anthropic-version"], "2023-06-01");
        assert_eq!(h["anthropic-beta"], "prompt-caching-2024-07-31");

        let mut anthropic_keyed = anthropic.clone();
        anthropic_keyed.auth_type = Some("x-api-key".into());
        let h = build_provider_upstream_headers(&anthropic_keyed, &req);
        assert_eq!(h["x-api-key"], "sk-test");
        assert!(!h.contains_key("authorization"));
    }

    #[test]
    fn non_anthropic_provider_does_not_forward_anthropic_headers() {
        let (_, openai) = provider("oai", "https://api.openai.com", "openai");
        let mut req = BTreeMap::new();
        req.insert("anthropic-version".into(), "2023-06-01".into());
        req.insert("anthropic-beta".into(), "prompt-caching".into());
        req.insert("accept".into(), "application/json".into());

        let h = build_provider_upstream_headers(&openai, &req);
        assert!(!h.contains_key("anthropic-version"));
        assert!(!h.contains_key("anthropic-beta"));
        assert_eq!(h["accept"], "application/json", "shared header still forwarded");
    }

    #[test]
    fn resolve_url_appends_query_only_when_present() {
        assert_eq!(
            resolve_provider_request_url("https://x/y/", "", "/v1/messages"),
            "https://x/y/v1/messages"
        );
        assert_eq!(
            resolve_provider_request_url("https://x/y", "a=1&b=2", "/v1/alpha/search"),
            "https://x/y/v1/alpha/search?a=1&b=2"
        );
    }

    #[test]
    fn strip_response_headers_drops_hop_by_hop_only() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("content-length", "10".parse().unwrap());
        headers.insert("transfer-encoding", "chunked".parse().unwrap());
        headers.insert("content-type", "text/event-stream".parse().unwrap());
        headers.insert("request-id", "abc".parse().unwrap());
        let out = strip_response_headers(headers);
        assert!(out.get("content-length").is_none());
        assert!(out.get("transfer-encoding").is_none());
        assert_eq!(out.get("content-type").unwrap(), "text/event-stream");
        assert_eq!(out.get("request-id").unwrap(), "abc");
    }
}