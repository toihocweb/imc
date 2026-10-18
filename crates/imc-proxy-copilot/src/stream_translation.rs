//! Port of `main/proxy/services/copilot/stream_translation.py` — OpenAI Chat
//! Completion chunks -> Anthropic Messages SSE events.
//!
//! Python returns a `List[Dict[str, Any]]` per chunk and the route wraps each
//! entry in an SSE frame; events are therefore modelled here as
//! `Vec<serde_json::Value>` in exactly that shape. [`format_sse_event`] renders
//! one event into the frame format `routes/messages.py` emits verbatim.
//!
//! ## Locally-defined types
//!
//! [`AnthropicStreamState`] and [`AnthropicStreamStateToolCall`] mirror the
//! plain Python classes declared *in this module* (snake_case attributes, an
//! int-keyed tool-call map), which are not the same shape as the
//! `imc-proxy-types` structs of those names.

use std::collections::BTreeMap;

use imc_proxy_types::types::chat_completions::{ChatCompletionChunk, Choice, Delta};
use serde_json::{json, Map, Value};

/// Per-stream bookkeeping — the port of the module-level
/// `AnthropicStreamState` class.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnthropicStreamState {
    pub message_start_sent: bool,
    pub content_block_open: bool,
    pub content_block_index: i64,
    pub thinking_block_open: bool,
    /// Keyed by the OpenAI `tool_call.index`.
    pub tool_calls: BTreeMap<i64, AnthropicStreamStateToolCall>,
    pub deferred_content: Option<String>,
    pub pending_message_delta: Option<Value>,
    pub message_stop_sent: bool,
}

/// Port of the anonymous per-tool-call dict in `handle_tool_calls`
/// (`{"id", "name", "anthropicBlockIndex"}`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnthropicStreamStateToolCall {
    pub id: String,
    pub name: String,
    pub anthropic_block_index: i64,
}

/// Port of `map_stop_reason` — OpenAI finish reasons to Anthropic stop reasons.
pub fn map_stop_reason(reason: Option<&str>) -> Option<String> {
    let reason = reason.filter(|r| !r.is_empty())?;
    Some(
        match reason {
            "stop" => "end_turn",
            "length" => "max_tokens",
            "tool_calls" => "tool_use",
            "content_filter" => "stop_sequence",
            other => other,
        }
        .to_string(),
    )
}

/// Port of `get_usage`.
///
/// Anthropic bills cached prompt tokens separately from fresh ones, so the
/// cached portion is subtracted out of `input_tokens`. The two cache counters
/// are only emitted when non-zero, matching Python's conditional `if ct > 0`.
pub fn get_usage(chunk: &ChatCompletionChunk) -> Value {
    let mut usage = Map::new();

    if let Some(u) = &chunk.usage {
        let pt = u.prompt_tokens;
        let details = u.prompt_tokens_details.as_ref();
        let ct = details.and_then(|d| d.cached_tokens).unwrap_or(0);
        let ct_write = details.and_then(|d| d.cache_write_tokens).unwrap_or(0);

        usage.insert("input_tokens".to_string(), json!(std::cmp::max(0, pt - ct)));
        usage.insert("output_tokens".to_string(), json!(u.completion_tokens));
        if ct > 0 {
            usage.insert("cache_read_input_tokens".to_string(), json!(ct));
        }
        if ct_write > 0 {
            usage.insert("cache_creation_input_tokens".to_string(), json!(ct_write));
        }
    }

    Value::Object(usage)
}

/// Port of `handle_message_start` — emits `message_start` exactly once.
pub fn handle_message_start(
    state: &mut AnthropicStreamState,
    events: &mut Vec<Value>,
    chunk: &ChatCompletionChunk,
) {
    if state.message_start_sent {
        return;
    }

    events.push(json!({
        "type": "message_start",
        "message": {
            "id": chunk.id,
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": chunk.model,
            "stop_reason": null,
            "stop_sequence": null,
            "usage": get_usage(chunk),
        }
    }));

    state.message_start_sent = true;
}

/// Port of `handle_thinking_text`.
///
/// Reasoning arriving while a text block is already open is *retyped* into
/// text on the delta, because Anthropic streams content blocks sequentially —
/// the caller ([`handle_content`]) then sees it as ordinary content. The
/// mutation is on the shared delta, so this takes `&mut Delta`.
pub fn handle_thinking_text(
    delta: &mut Delta,
    state: &mut AnthropicStreamState,
    events: &mut Vec<Value>,
) {
    let reasoning_text = delta
        .reasoning_text
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| delta.reasoning_content.clone().filter(|s| !s.is_empty()));

    let Some(reasoning_text) = reasoning_text else {
        return;
    };

    if state.content_block_open {
        // Fallback if reasoning arrives while content open.
        delta.content = Some(reasoning_text);
        delta.reasoning_text = None;
        delta.reasoning_content = None;
        return;
    }

    if !state.thinking_block_open {
        events.push(json!({
            "type": "content_block_start",
            "index": state.content_block_index,
            "content_block": {"type": "thinking", "thinking": ""}
        }));
        state.thinking_block_open = true;
    }

    events.push(json!({
        "type": "content_block_delta",
        "index": state.content_block_index,
        "delta": {"type": "thinking_delta", "thinking": reasoning_text}
    }));
}

/// Port of `close_thinking_block` — closes the open thinking block with an
/// empty signature so Copilot's opaque-signature round trip still terminates
/// the block.
pub fn close_thinking_block(state: &mut AnthropicStreamState, events: &mut Vec<Value>) {
    if !state.thinking_block_open {
        return;
    }

    events.push(json!({
        "type": "content_block_delta",
        "index": state.content_block_index,
        "delta": {"type": "signature_delta", "signature": ""}
    }));
    events.push(json!({
        "type": "content_block_stop",
        "index": state.content_block_index
    }));
    state.content_block_index += 1;
    state.thinking_block_open = false;
}

/// Port of `handle_content` — text deltas, tool-deferral, and the opaque
/// signature that terminates a thinking block.
pub fn handle_content(
    delta: &Delta,
    state: &mut AnthropicStreamState,
    events: &mut Vec<Value>,
) {
    let content = delta.content.clone().unwrap_or_default();

    if !content.is_empty() {
        close_thinking_block(state, events);

        let is_tool_open = state.content_block_open
            && state
                .tool_calls
                .values()
                .any(|tc| tc.anthropic_block_index == state.content_block_index);
        let has_tool_calls = delta.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());

        if is_tool_open || has_tool_calls {
            // Text that follows a tool call must land *after* the tool_use
            // block, so it is buffered until the tool block closes.
            state.deferred_content = Some(format!(
                "{}{}",
                state.deferred_content.take().unwrap_or_default(),
                content
            ));
        } else {
            if !state.content_block_open {
                events.push(json!({
                    "type": "content_block_start",
                    "index": state.content_block_index,
                    "content_block": {"type": "text", "text": ""}
                }));
                state.content_block_open = true;
            }

            events.push(json!({
                "type": "content_block_delta",
                "index": state.content_block_index,
                "delta": {"type": "text_delta", "text": content}
            }));
        }

        // Python `return`s above, so the signature branch below is skipped.
        return;
    }

    // An empty content plus an opaque signature is how Copilot closes a
    // thinking block that streamed its reasoning separately. Python compares
    // against the literal `""`, so an absent content does *not* qualify.
    if delta.content.as_deref() == Some("") {
        if let Some(signature) = delta.reasoning_opaque.as_ref().filter(|s| !s.is_empty()) {
            if state.thinking_block_open {
                events.push(json!({
                    "type": "content_block_delta",
                    "index": state.content_block_index,
                    "delta": {"type": "signature_delta", "signature": signature}
                }));
                events.push(json!({
                    "type": "content_block_stop",
                    "index": state.content_block_index
                }));
                state.content_block_index += 1;
                state.thinking_block_open = false;
            }
        }
    }
}

/// Port of `handle_tool_calls` — opens a `tool_use` block per tool call and
/// streams its JSON arguments as `input_json_delta`.
pub fn handle_tool_calls(
    delta: &Delta,
    state: &mut AnthropicStreamState,
    events: &mut Vec<Value>,
) {
    let Some(tool_calls) = delta.tool_calls.as_ref() else {
        return;
    };

    for tc in tool_calls {
        let has_id_and_name = tc
            .id
            .as_deref()
            .map_or(false, |id| !id.is_empty())
            && tc.function
                .as_ref()
                .and_then(|f| f.name.as_deref())
                .map_or(false, |n| !n.is_empty());

        if has_id_and_name {
            if state.content_block_open {
                events.push(json!({
                    "type": "content_block_stop",
                    "index": state.content_block_index
                }));
                state.content_block_index += 1;
                state.content_block_open = false;
            }

            let anthropic_idx = state.content_block_index;
            let name = tc.function.as_ref().and_then(|f| f.name.clone()).unwrap_or_default();
            state.tool_calls.insert(
                tc.index,
                AnthropicStreamStateToolCall {
                    id: tc.id.clone().unwrap_or_default(),
                    name: name.clone(),
                    anthropic_block_index: anthropic_idx,
                },
            );

            events.push(json!({
                "type": "content_block_start",
                "index": anthropic_idx,
                "content_block": {
                    "type": "tool_use",
                    "id": tc.id,
                    "name": name,
                    "input": {}
                }
            }));
            state.content_block_open = true;
        }

        if let Some(arguments) = tc
            .function
            .as_ref()
            .and_then(|f| f.arguments.as_deref())
            .filter(|a| !a.is_empty())
        {
            if let Some(tc_info) = state.tool_calls.get(&tc.index) {
                let block_index = tc_info.anthropic_block_index;
                events.push(json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {"type": "input_json_delta", "partial_json": arguments}
                }));
            }
        }
    }
}

/// Port of `flush_deferred_content` — emits the buffered text in its own,
/// immediately-closed block after the tool calls that preceded it.
pub fn flush_deferred_content(state: &mut AnthropicStreamState, events: &mut Vec<Value>) {
    let Some(deferred) = state.deferred_content.clone().filter(|d| !d.is_empty()) else {
        return;
    };

    if !state.content_block_open {
        events.push(json!({
            "type": "content_block_start",
            "index": state.content_block_index,
            "content_block": {"type": "text", "text": ""}
        }));
        state.content_block_open = true;
    }

    events.push(json!({
        "type": "content_block_delta",
        "index": state.content_block_index,
        "delta": {"type": "text_delta", "text": deferred}
    }));
    events.push(json!({
        "type": "content_block_stop",
        "index": state.content_block_index
    }));

    state.deferred_content = None;
    state.content_block_open = false;
    state.content_block_index += 1;
}

/// Port of `handle_finish` — closes open blocks and stashes the
/// `message_delta` for the next usage-only chunk.
pub fn handle_finish(
    choice: &Choice,
    state: &mut AnthropicStreamState,
    events: &mut Vec<Value>,
    chunk: &ChatCompletionChunk,
) {
    let Some(finish_reason) = choice.finish_reason.as_deref().filter(|r| !r.is_empty()) else {
        return;
    };

    if state.content_block_open {
        events.push(json!({
            "type": "content_block_stop",
            "index": state.content_block_index
        }));
        state.content_block_open = false;
        state.content_block_index += 1;
    }

    flush_deferred_content(state, events);

    // Some upstreams (e.g. Gemini via litellm) report finish_reason="stop"
    // even when the turn emitted tool calls. Anthropic clients require
    // stop_reason="tool_use" to dispatch the tool, so override when any
    // tool call was seen this stream.
    let mut mapped_stop = map_stop_reason(Some(finish_reason));
    if !state.tool_calls.is_empty() && mapped_stop.as_deref() == Some("end_turn") {
        mapped_stop = Some("tool_use".to_string());
    }

    state.pending_message_delta = Some(json!({
        "type": "message_delta",
        "delta": {
            "stop_reason": mapped_stop,
            "stop_sequence": null
        },
        "usage": get_usage(chunk)
    }));
}

/// Port of `translate_chunk_to_anthropic_events`.
///
/// A chunk with no `choices` is the trailing usage-only chunk: it is where the
/// stashed `message_delta` gets its real numbers and `message_stop` is
/// finally emitted.
pub fn translate_chunk_to_anthropic_events(
    chunk: &ChatCompletionChunk,
    state: &mut AnthropicStreamState,
) -> Vec<Value> {
    let mut events: Vec<Value> = Vec::new();

    if chunk.choices.is_empty() {
        if chunk.usage.is_some() && state.pending_message_delta.is_some() {
            if let Some(pending) = state.pending_message_delta.as_mut() {
                if let Some(obj) = pending.as_object_mut() {
                    obj.insert("usage".to_string(), get_usage(chunk));
                }
            }
            if let Some(pending) = state.pending_message_delta.take() {
                events.push(pending);
            }
            events.push(json!({"type": "message_stop"}));
            state.message_stop_sent = true;
        }
        return events;
    }

    let choice = &chunk.choices[0];
    let mut delta = choice.delta.clone();

    handle_message_start(state, &mut events, chunk);
    handle_thinking_text(&mut delta, state, &mut events);
    handle_content(&delta, state, &mut events);
    handle_tool_calls(&delta, state, &mut events);
    handle_finish(choice, state, &mut events, chunk);

    events
}

/// Render one event as the Anthropic SSE frame `routes/messages.py` emits:
/// `event: <type>\ndata: <json>\n\n`.
///
/// The `event:` line is mandatory — Claude Code's parser buffers the whole
/// stream without it.
pub fn format_sse_event(event: &Value) -> String {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    format!("event: {event_type}\ndata: {data}\n\n")
}