//! Port of `main/proxy/services/copilot/responses_stream_translation.py` —
//! OpenAI Responses SSE events -> Anthropic Messages SSE events.
//!
//! Python routes each raw event through a stateful translator that yields a
//! list of Anthropic events; events are modelled here as
//! `Vec<serde_json::Value>`. [`crate::stream_translation::format_sse_event`]
//! renders them into the frame format the route emits.
//!
//! ## Locally-defined types
//!
//! [`ResponsesStreamState`] and [`FunctionCallStreamState`] mirror the plain
//! Python classes declared *in this module*, which are distinct from the
//! `imc-proxy-types` structs of the same names (those carry wire-rename
//! attributes for a different consumer).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

/// Port of the `FunctionCallStreamState` class — one in-flight function call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FunctionCallStreamState {
    pub block_index: i64,
    pub tool_call_id: String,
    pub name: String,
}

/// Port of the `ResponsesStreamState` class.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResponsesStreamState {
    pub message_start_sent: bool,
    pub message_completed: bool,
    pub next_content_block_index: i64,
    /// Keyed by `"{output_index}:{content_index}"`.
    pub block_index_by_key: BTreeMap<String, i64>,
    pub open_blocks: BTreeSet<i64>,
    pub block_has_delta: BTreeSet<i64>,
    pub function_call_state_by_output_index: BTreeMap<i64, FunctionCallStreamState>,
    pub reasoning_summary_has_delta: BTreeSet<String>,
    pub has_tool_call: bool,
}

/// Port of `translate_responses_stream_event`.
pub fn translate_responses_stream_event(
    raw_event: &Value,
    state: &mut ResponsesStreamState,
) -> Vec<Value> {
    let mut events: Vec<Value> = Vec::new();
    let event_type = raw_event.get("type").and_then(Value::as_str).unwrap_or("");

    match event_type {
        "response.created" => {
            state.message_start_sent = true;
            let response_obj = raw_event.get("response").cloned().unwrap_or(json!({}));
            events.push(json!({
                "type": "message_start",
                "message": {
                    "id": response_obj.get("id").cloned().unwrap_or(Value::Null),
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": response_obj.get("model").cloned().unwrap_or(Value::Null),
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0}
                }
            }));
        }
        "response.output_item.added" => {
            let item = raw_event.get("item").cloned().unwrap_or(json!({}));
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let block_index = open_function_call_block(
                    state,
                    raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0),
                    item.get("call_id").and_then(Value::as_str),
                    item.get("name").and_then(Value::as_str),
                    &mut events,
                );
                if let Some(arguments) =
                    item.get("arguments").and_then(Value::as_str).filter(|a| !a.is_empty())
                {
                    events.push(json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {"type": "input_json_delta", "partial_json": arguments}
                    }));
                    state.block_has_delta.insert(block_index);
                }
            }
        }
        "response.output_text.delta" => {
            let block_index = open_text_block_if_needed(
                state,
                raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0),
                raw_event.get("content_index").and_then(Value::as_i64).unwrap_or(0),
                &mut events,
            );
            if let Some(delta) =
                raw_event.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty())
            {
                events.push(json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {"type": "text_delta", "text": delta}
                }));
                state.block_has_delta.insert(block_index);
            }
        }
        "response.reasoning_summary_text.delta" => {
            let output_index = raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0);
            let block_index = open_thinking_block_if_needed(state, output_index, &mut events);
            if let Some(delta) =
                raw_event.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty())
            {
                events.push(json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {"type": "thinking_delta", "thinking": delta}
                }));
                state.block_has_delta.insert(block_index);
                state.reasoning_summary_has_delta.insert(format!("{output_index}:0"));
            }
        }
        "response.output_text.done" => {
            let output_index = raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0);
            let content_index = raw_event
                .get("content_index")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let block_index = open_text_block_if_needed(state, output_index, content_index, &mut events);
            if let Some(text) = raw_event.get("text").and_then(Value::as_str).filter(|t| !t.is_empty())
            {
                if !state.block_has_delta.contains(&block_index) {
                    events.push(json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {"type": "text_delta", "text": text}
                    }));
                }
            }
        }
        "response.function_call_arguments.delta" => {
            let output_index = raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0);
            let block_index = open_function_call_block(state, output_index, None, None, &mut events);
            if let Some(delta) =
                raw_event.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty())
            {
                events.push(json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {"type": "input_json_delta", "partial_json": delta}
                }));
                state.block_has_delta.insert(block_index);
            }
        }
        "response.function_call_arguments.done" => {
            let output_index = raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0);
            let block_index = open_function_call_block(state, output_index, None, None, &mut events);
            if let Some(arguments) = raw_event
                .get("arguments")
                .and_then(Value::as_str)
                .filter(|a| !a.is_empty())
            {
                if !state.block_has_delta.contains(&block_index) {
                    events.push(json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {"type": "input_json_delta", "partial_json": arguments}
                    }));
                    state.block_has_delta.insert(block_index);
                }
            }
            state.function_call_state_by_output_index.remove(&output_index);
        }
        "response.output_item.done" => {
            let output_index = raw_event.get("output_index").and_then(Value::as_i64).unwrap_or(0);
            let item = raw_event.get("item").cloned().unwrap_or(json!({}));
            if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                let block_index = open_thinking_block_if_needed(state, output_index, &mut events);
                // Copilot encodes its reasoning seal as
                // `<encrypted_content>@<item id>`; a bare `@` means neither was
                // supplied and the block is left unsigned.
                let signature = format!(
                    "{}@{}",
                    item.get("encrypted_content").and_then(Value::as_str).unwrap_or(""),
                    item.get("id").and_then(Value::as_str).unwrap_or("")
                );
                if signature != "@" {
                    events.push(json!({
                        "type": "content_block_delta",
                        "index": block_index,
                        "delta": {"type": "signature_delta", "signature": signature}
                    }));
                    state.block_has_delta.insert(block_index);
                }
            }
        }
        "response.completed" | "response.incomplete" => {
            close_all_open_blocks(state, &mut events);
            let stop_reason = if event_type == "response.incomplete" {
                "max_tokens"
            } else {
                "end_turn"
            };

            let usage_obj = raw_event
                .get("response")
                .and_then(|r| r.get("usage"))
                .cloned()
                .unwrap_or(json!({}));
            let input_details = usage_obj
                .get("input_tokens_details")
                .cloned()
                .unwrap_or(json!({}));

            events.push(json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                "usage": {
                    "cache_creation_input_tokens": json_int(&input_details, "cache_write_tokens"),
                    "cache_read_input_tokens": json_int(&input_details, "cached_tokens"),
                    "input_tokens": json_int(&usage_obj, "input_tokens"),
                    "output_tokens": json_int(&usage_obj, "output_tokens"),
                }
            }));
            events.push(json!({"type": "message_stop"}));
            state.message_completed = true;
        }
        "response.failed" => {
            close_all_open_blocks(state, &mut events);
            events.push(json!({
                "type": "error",
                "error": {"type": "api_error", "message": "Response failed"}
            }));
            state.message_completed = true;
        }
        "error" => {
            close_all_open_blocks(state, &mut events);
            let message = raw_event
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Error");
            events.push(json!({
                "type": "error",
                "error": {"type": "api_error", "message": message}
            }));
            state.message_completed = true;
        }
        "ping" => {
            events.push(json!({"type": "ping"}));
        }
        _ => {}
    }

    events
}

/// Python's `d.get(key, 0)` for an integer usage counter.
fn json_int(obj: &Value, key: &str) -> i64 {
    obj.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// Port of `_open_text_block_if_needed` — opens a `text` block for one
/// `output_index:content_index` pair, closing whichever block is currently
/// open (Anthropic streams blocks strictly sequentially).
fn open_text_block_if_needed(
    state: &mut ResponsesStreamState,
    output_index: i64,
    content_index: i64,
    events: &mut Vec<Value>,
) -> i64 {
    let key = format!("{output_index}:{content_index}");
    let block_index = if let Some(idx) = state.block_index_by_key.get(&key) {
        *idx
    } else {
        let idx = state.next_content_block_index;
        state.next_content_block_index += 1;
        state.block_index_by_key.insert(key, idx);
        idx
    };

    if !state.open_blocks.contains(&block_index) {
        close_open_blocks(state, events);
        events.push(json!({
            "type": "content_block_start",
            "index": block_index,
            "content_block": {"type": "text", "text": ""}
        }));
        state.open_blocks.insert(block_index);
    }

    block_index
}

/// Port of `_open_thinking_block_if_needed`.
fn open_thinking_block_if_needed(
    state: &mut ResponsesStreamState,
    output_index: i64,
    events: &mut Vec<Value>,
) -> i64 {
    let key = format!("{output_index}:0");
    let block_index = if let Some(idx) = state.block_index_by_key.get(&key) {
        *idx
    } else {
        let idx = state.next_content_block_index;
        state.next_content_block_index += 1;
        state.block_index_by_key.insert(key, idx);
        idx
    };

    if !state.open_blocks.contains(&block_index) {
        close_open_blocks(state, events);
        events.push(json!({
            "type": "content_block_start",
            "index": block_index,
            "content_block": {"type": "thinking", "thinking": ""}
        }));
        state.open_blocks.insert(block_index);
    }

    block_index
}

/// Port of `_open_function_call_block`.
///
/// A function call is identified by its `output_index` alone, so its block
/// index stays stable across the `.added` and `.delta` events that reference
/// it — the id/name may only be present on the first of those.
fn open_function_call_block(
    state: &mut ResponsesStreamState,
    output_index: i64,
    tool_call_id: Option<&str>,
    name: Option<&str>,
    events: &mut Vec<Value>,
) -> i64 {
    state.has_tool_call = true;

    if !state
        .function_call_state_by_output_index
        .contains_key(&output_index)
    {
        let block_index = state.next_content_block_index;
        state.next_content_block_index += 1;
        state.function_call_state_by_output_index.insert(
            output_index,
            FunctionCallStreamState {
                block_index,
                tool_call_id: tool_call_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("tool_call_{block_index}")),
                name: name.unwrap_or("function").to_string(),
            },
        );
    }

    let func_state = &state.function_call_state_by_output_index[&output_index];
    let block_index = func_state.block_index;
    let tool_call_id = func_state.tool_call_id.clone();
    let fn_name = func_state.name.clone();

    if !state.open_blocks.contains(&block_index) {
        close_open_blocks(state, events);
        events.push(json!({
            "type": "content_block_start",
            "index": block_index,
            "content_block": {
                "type": "tool_use",
                "id": tool_call_id,
                "name": fn_name,
                "input": {}
            }
        }));
        state.open_blocks.insert(block_index);
    }

    block_index
}

/// Port of `_close_open_blocks` — Anthropic allows only one open content
/// block, so closing is all-or-nothing.
fn close_open_blocks(state: &mut ResponsesStreamState, events: &mut Vec<Value>) {
    let open: Vec<i64> = state.open_blocks.iter().copied().collect();
    for block_index in open {
        events.push(json!({"type": "content_block_stop", "index": block_index}));
        state.open_blocks.remove(&block_index);
        state.block_has_delta.remove(&block_index);
    }
}

/// Port of `_close_all_open_blocks` — also resets the per-call bookkeeping so
/// a follow-up response starts clean.
fn close_all_open_blocks(state: &mut ResponsesStreamState, events: &mut Vec<Value>) {
    close_open_blocks(state, events);
    state.function_call_state_by_output_index.clear();
    state.reasoning_summary_has_delta.clear();
}