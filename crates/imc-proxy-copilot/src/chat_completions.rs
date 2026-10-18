//! Port of `main/proxy/services/copilot/chat_completions.py` — POST an OpenAI
//! Chat Completions payload to Copilot.
//!
//! ## Port notes
//!
//! * The Python's `get_copilot_token()` lives in
//!   `main/proxy/services/github/`, i.e. the sibling `imc-proxy` crate, which
//!   depends on *this* crate. To keep the dependency graph acyclic the token
//!   fetch is injected here as [`CopilotTokenProvider`]; the "only fetch when
//!   the cached token is missing" behaviour is preserved verbatim.
//! * Python's `_sanitize_message_content` mutates the dumped body dict before
//!   it is sent. [`sanitize_message_content`] does the same on a
//!   `serde_json::Value`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use imc_proxy_types::api_config::{copilot_base_url, github_copilot_headers};
use imc_proxy_types::state::{state, with_state_mut};
use imc_proxy_types::types::chat_completions::ChatCompletionsPayload;
use serde_json::Value;

use crate::non_stream_translation::DEFAULT_EMPTY_CONTENT;

/// The JSON body `get_copilot_token()` returns in Python.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TokenData {
    pub token: Option<String>,
}

/// Port of the injected `get_copilot_token` import.
pub trait CopilotTokenProvider: Send + Sync {
    /// Refresh the Copilot token from GitHub. Resolves the token string
    /// directly so callers cannot forget to persist it into `state`.
    fn get_copilot_token(&self) -> BoxFuture<'_, Result<String>>;
}

/// Port of the module-level `if not state.copilotToken:` guard that precedes
/// each upstream call: reuse the cached token, refresh only when absent.
pub async fn ensure_copilot_token<P: CopilotTokenProvider + ?Sized>(provider: &P) -> Result<()> {
    if !state().copilot_token.as_deref().is_some_and(|t| !t.is_empty()) {
        let token = provider
            .get_copilot_token()
            .await
            .context("failed to get Copilot token")?;
        with_state_mut(|s| s.copilot_token = Some(token));
    }
    Ok(())
}

/// Port of `_sanitize_message_content`.
///
/// Guards against `content: null` on messages that are not assistant
/// tool-call turns. OpenAI/Copilot only accept null content when an assistant
/// message carries `tool_calls`; otherwise it returns 400 "expected a string,
/// got null".
pub fn sanitize_message_content(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };
        if !obj.get("content").map(Value::is_null).unwrap_or(false) {
            continue;
        }
        let is_assistant_with_tool_calls = obj.get("role").and_then(Value::as_str) == Some("assistant")
            && obj
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|t| !t.is_empty())
                .unwrap_or(false);
        if is_assistant_with_tool_calls {
            continue;
        }
        obj.insert(
            "content".to_string(),
            Value::String(DEFAULT_EMPTY_CONTENT.to_string()),
        );
    }
}

/// Port of `chat_completions` — build the body exactly as
/// `model_dump(exclude_none=True)` would, sanitize it, then POST it.
pub async fn chat_completions<P: CopilotTokenProvider + ?Sized>(
    provider: &P,
    payload: &ChatCompletionsPayload,
) -> Result<reqwest::Response> {
    // 1. Ensure we have a valid token.
    ensure_copilot_token(provider).await?;

    let st = state();
    let url = format!("{}/chat/completions", copilot_base_url(&st));
    let headers = github_copilot_headers(&st, None, false);

    let mut body = serde_json::to_value(payload).context("serialize chat completions payload")?;
    // Python: `payload.model_dump(exclude_none=True)` — serde's
    // `skip_serializing_if = "Option::is_none"` on every field does the same.
    sanitize_message_content(&mut body);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("build http client")?;

    // 2. Send request to Copilot.
    let request = client
        .post(&url)
        .headers(header_map(&headers))
        .json(&body)
        .build()
        .context("build chat completions request")?;

    let response = client.execute(request).await.context("post chat completions")?;
    Ok(response)
}

/// Convert the Python-style `Dict[str, str]` headers into a reqwest header
/// map, dropping any name that fails to parse (Python would have raised, but
/// every name here is a known-safe literal).
pub fn header_map(headers: &BTreeMap<String, String>) -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            map.insert(name, value);
        }
    }
    map
}

/// The `x-initiator` header the Copilot API uses to distinguish a request the
/// user typed from one an agent generated.
pub const INITIATOR_USER: &str = "user";

/// The placeholder written when Python's `content is None` reached the body.
pub fn empty_content_placeholder() -> &'static str {
    DEFAULT_EMPTY_CONTENT
}