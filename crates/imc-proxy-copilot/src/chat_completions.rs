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
/// Guards against:
/// 1. `content: null` on messages that are not assistant tool-call turns.
/// 2. Missing `name` on `role: "tool"` messages, which Google Gemini's protobuf
///    `FunctionResponse` strictly requires (`REQUIRED_FIELD_MISSING`).
pub fn sanitize_message_content(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    // 1. Collect tool names from assistant tool_calls
    let mut tool_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for msg in messages.iter() {
        if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                if let (Some(id), Some(name)) = (
                    tc.get("id").and_then(Value::as_str),
                    tc.get("function").and_then(|f| f.get("name")).and_then(Value::as_str),
                ) {
                    if !id.is_empty() && !name.is_empty() {
                        tool_names.insert(id.to_string(), name.to_string());
                    }
                }
            }
        }
    }

    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };

        // Ensure tool messages have the required 'name' field for Gemini and other backends
        if obj.get("role").and_then(Value::as_str) == Some("tool") {
            let has_name = obj.get("name").and_then(Value::as_str).map_or(false, |s| !s.is_empty());
            if !has_name {
                let tool_call_id = obj.get("tool_call_id").and_then(Value::as_str).unwrap_or("");
                let name = tool_names
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        if !tool_call_id.is_empty() {
                            tool_call_id.to_string()
                        } else {
                            "unknown_function".to_string()
                        }
                    });
                obj.insert("name".to_string(), Value::String(name));
            }
        }

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

    // 2. Normalize message ordering and tail:
    // Google Gemini (and OpenAI backends) reject requests ending with a model/assistant turn:
    // "Requests ending with a model turn are not supported."
    // - Separate system messages to the beginning.
    // - Ensure conversation does not end on an assistant turn (pop empty, append user 'Continue' if non-empty).
    let mut system_msgs: Vec<Value> = Vec::new();
    let mut conv_msgs: Vec<Value> = Vec::new();
    for msg in messages.drain(..) {
        if msg.get("role").and_then(Value::as_str) == Some("system") {
            system_msgs.push(msg);
        } else {
            conv_msgs.push(msg);
        }
    }

    while let Some(last) = conv_msgs.last() {
        let role = last.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "assistant" {
            let has_tools = last
                .get("tool_calls")
                .and_then(Value::as_array)
                .map_or(false, |t| !t.is_empty());
            let content_str = last
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let is_placeholder_or_empty = content_str.is_empty()
                || content_str == DEFAULT_EMPTY_CONTENT
                || content_str == "thinking...";

            if !has_tools && is_placeholder_or_empty {
                conv_msgs.pop();
                continue;
            } else {
                conv_msgs.push(serde_json::json!({
                    "role": "user",
                    "content": "Continue"
                }));
                break;
            }
        } else {
            break;
        }
    }

    messages.extend(system_msgs);
    messages.extend(conv_msgs);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_message_content_populates_missing_tool_name_from_assistant_tool_calls() {
        let mut body = json!({
            "messages": [
                {"role": "user", "content": "run ls"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_123", "type": "function", "function": {"name": "Bash", "arguments": "{\"command\":\"ls\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_123", "content": "file1.txt"}
            ]
        });

        sanitize_message_content(&mut body);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[2]["name"], "Bash");
    }

    #[test]
    fn sanitize_message_content_falls_back_when_id_unknown() {
        let mut body = json!({
            "messages": [
                {"role": "tool", "tool_call_id": "call_unknown", "content": "result"}
            ]
        });

        sanitize_message_content(&mut body);

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["name"], "call_unknown");
    }

    #[test]
    fn sanitize_message_content_prevents_ending_with_assistant_turn() {
        // Subcase 1: Non-empty assistant turn gets 'Continue' appended
        let mut body = json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "I am an assistant"}
            ]
        });
        sanitize_message_content(&mut body);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"], "Continue");

        // Subcase 2: Empty assistant turn is removed
        let mut body2 = json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": ""}
            ]
        });
        sanitize_message_content(&mut body2);
        let msgs2 = body2["messages"].as_array().unwrap();
        assert_eq!(msgs2.len(), 1);
        assert_eq!(msgs2[0]["role"], "user");

        // Subcase 3: Trailing system message is moved to front and assistant is followed by user
        let mut body3 = json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "hi"},
                {"role": "system", "content": "environment info"}
            ]
        });
        sanitize_message_content(&mut body3);
        let msgs3 = body3["messages"].as_array().unwrap();
        assert_eq!(msgs3[0]["role"], "system");
        assert_eq!(msgs3.last().unwrap()["role"], "user");
    }
}