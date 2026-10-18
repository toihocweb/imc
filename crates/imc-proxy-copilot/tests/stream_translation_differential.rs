//! Differential test: the chat-completion stream translator must emit exactly
//! the same Anthropic event sequence as the Python implementation.
//!
//! The expected event lists below were produced by running
//! `main/proxy/services/copilot/stream_translation.py` over the same chunk
//! sequences. This is the part of the port where a subtle deviation in
//! `content_block_start` / `content_block_delta` / `message_stop` ordering or
//! index bookkeeping silently breaks a streaming agent rather than erroring,
//! so the sequences are compared as whole values, not field by field.

use imc_proxy_copilot::stream_translation::{
    translate_chunk_to_anthropic_events, AnthropicStreamState,
};
use serde_json::{json, Value};

/// Feed a chunk sequence through the translator and collect every event.
fn run(chunks: &[Value]) -> Vec<Value> {
    let mut state = AnthropicStreamState::default();
    let mut events: Vec<Value> = Vec::new();
    for c in chunks {
        let chunk: imc_proxy_types::types::chat_completions::ChatCompletionChunk =
            serde_json::from_value(c.clone()).expect("valid chunk");
        events.extend(translate_chunk_to_anthropic_events(&chunk, &mut state));
    }
    events
}

fn assert_events(chunks: &[Value], expected: Value, case: &str) {
    let actual = run(chunks);
    assert_eq!(
        json!(actual),
        expected,
        "event sequence mismatch in case {case}"
    );
}

fn choice(delta: Value, finish: Option<&str>) -> Value {
    json!({"index": 0, "delta": delta, "finish_reason": finish})
}

#[test]
fn plain_text_stream_matches_python() {
    let chunks = vec![
        json!({"id": "chatcmpl-1", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"role": "assistant", "content": "Hel"}), None)]}),
        json!({"id": "chatcmpl-1", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"content": "lo!"}), None)]}),
        json!({"id": "chatcmpl-1", "model": "gpt-5.3-codex",
               "choices": [choice(json!({}), Some("stop"))]}),
        // The trailing usage-only chunk is where message_delta gets its real
        // numbers and message_stop is emitted.
        json!({"id": "chatcmpl-1", "model": "gpt-5.3-codex", "choices": [],
               "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13,
                         "prompt_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 2,
                                                    "cache_creation_input_tokens": 0,
                                                    "accepted_prediction_tokens": 0,
                                                    "rejected_prediction_tokens": 0}}}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "chatcmpl-1", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "Hel"}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "lo!"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null},
             "usage": {"input_tokens": 6, "output_tokens": 3,
                       "cache_read_input_tokens": 4, "cache_creation_input_tokens": 2}},
            {"type": "message_stop"}
        ]),
        "plain_text",
    );
}

#[test]
fn tool_call_then_text_defers_text_into_a_later_block() {
    let chunks = vec![
        json!({"id": "c2", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"role": "assistant", "content": "Checking."}), None)]}),
        json!({"id": "c2", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
                                   "function": {"name": "Bash", "arguments": ""}}]}), None)]}),
        json!({"id": "c2", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 0,
                                   "function": {"arguments": "{\"command\":"}}]}), None)]}),
        json!({"id": "c2", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 0,
                                   "function": {"arguments": "\"ls\"}"}}]}), None)]}),
        // finish_reason "stop" despite tool calls — Python defers the text and
        // flushes it as its own block after the tool_use block closes.
        json!({"id": "c2", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"content": "done now"}), Some("stop"))]}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "c2", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "Checking."}},
            {"type": "content_block_stop", "index": 0},
            {"type": "content_block_start", "index": 1,
             "content_block": {"type": "tool_use", "id": "call_1", "name": "Bash", "input": {}}},
            {"type": "content_block_delta", "index": 1,
             "delta": {"type": "input_json_delta", "partial_json": "{\"command\":"}},
            {"type": "content_block_delta", "index": 1,
             "delta": {"type": "input_json_delta", "partial_json": "\"ls\"}"}},
            {"type": "content_block_stop", "index": 1},
            {"type": "content_block_start", "index": 2,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 2,
             "delta": {"type": "text_delta", "text": "done now"}},
            {"type": "content_block_stop", "index": 2}
        ]),
        "tool_call_then_text",
    );
}

#[test]
fn reasoning_then_content_closes_thinking_before_text() {
    let chunks = vec![
        json!({"id": "c3", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"role": "assistant", "reasoning_text": "thinking hard"}), None)]}),
        json!({"id": "c3", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"content": "answer"}), Some("stop"))]}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "c3", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "thinking", "thinking": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "thinking_delta", "thinking": "thinking hard"}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "signature_delta", "signature": ""}},
            {"type": "content_block_stop", "index": 0},
            {"type": "content_block_start", "index": 1,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 1,
             "delta": {"type": "text_delta", "text": "answer"}},
            {"type": "content_block_stop", "index": 1}
        ]),
        "reasoning_then_content",
    );
}

#[test]
fn opaque_signature_closes_thinking_block() {
    let chunks = vec![
        json!({"id": "c4", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"reasoning_content": "hmm"}), None)]}),
        // Empty content + opaque signature is Copilot's thinking-block closer.
        json!({"id": "c4", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"content": "", "reasoning_opaque": "sig-xyz"}), None)]}),
        json!({"id": "c4", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"content": "final"}), Some("stop"))]}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "c4", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "thinking", "thinking": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "thinking_delta", "thinking": "hmm"}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "signature_delta", "signature": "sig-xyz"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "content_block_start", "index": 1,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 1,
             "delta": {"type": "text_delta", "text": "final"}},
            {"type": "content_block_stop", "index": 1}
        ]),
        "opaque_signature",
    );
}

#[test]
fn reasoning_while_content_open_is_retyped_as_text() {
    let chunks = vec![
        json!({"id": "c5", "model": "gpt-5.3-codex",
               "choices": [choice(json!({"role": "assistant", "content": "abc"}), None)]}),
        // Reasoning arriving inside an open text block becomes a text_delta.
        json!({"id": "c5", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"reasoning_text": "mid-stream reasoning"}), None)]}),
        json!({"id": "c5", "model": "gpt-5.3-codex",
               "choices": [choice(json!({}), Some("stop"))]}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "c5", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "abc"}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "mid-stream reasoning"}},
            {"type": "content_block_stop", "index": 0}
        ]),
        "reasoning_while_content_open",
    );
}

#[test]
fn two_tool_calls_get_distinct_blocks() {
    let chunks = vec![
        json!({"id": "c6", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 0, "id": "a1", "type": "function",
                                   "function": {"name": "Bash", "arguments": "{}"}}]}), None)]}),
        json!({"id": "c6", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 1, "id": "a2", "type": "function",
                                   "function": {"name": "Read", "arguments": "{}"}}]}), None)]}),
        json!({"id": "c6", "model": "gpt-5.3-codex",
               "choices": [choice(json!({}), Some("tool_calls"))]}),
    ];

    assert_events(
        &chunks,
        json!([
            {"type": "message_start", "message": {"id": "c6", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "tool_use", "id": "a1", "name": "Bash", "input": {}}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "input_json_delta", "partial_json": "{}"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "content_block_start", "index": 1,
             "content_block": {"type": "tool_use", "id": "a2", "name": "Read", "input": {}}},
            {"type": "content_block_delta", "index": 1,
             "delta": {"type": "input_json_delta", "partial_json": "{}"}},
            {"type": "content_block_stop", "index": 1}
        ]),
        "two_tool_calls",
    );
}

#[test]
fn finish_reason_stops_the_block_but_message_delta_waits_for_usage() {
    // `content_filter` and `length` both close the block; without a trailing
    // usage-only chunk the message_delta stays stashed, so no message_stop.
    let filter = vec![json!({"id": "c7", "model": "gpt-5.3-codex",
        "choices": [choice(json!({"content": "x"}), Some("content_filter"))]})];
    assert_events(
        &filter,
        json!([
            {"type": "message_start", "message": {"id": "c7", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "x"}},
            {"type": "content_block_stop", "index": 0}
        ]),
        "content_filter",
    );

    let length = vec![json!({"id": "c8", "model": "gpt-5.3-codex",
        "choices": [choice(json!({"content": "cut off"}), Some("length"))]})];
    assert_events(
        &length,
        json!([
            {"type": "message_start", "message": {"id": "c8", "type": "message",
              "role": "assistant", "content": [], "model": "gpt-5.3-codex",
              "stop_reason": null, "stop_sequence": null, "usage": {}}},
            {"type": "content_block_start", "index": 0,
             "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0,
             "delta": {"type": "text_delta", "text": "cut off"}},
            {"type": "content_block_stop", "index": 0}
        ]),
        "max_tokens",
    );
}

#[test]
fn stop_reason_is_overridden_to_tool_use_when_a_tool_was_called() {
    // finish_reason "stop" but a tool call was seen -> clients need "tool_use"
    // to dispatch the tool.
    let chunks = vec![
        json!({"id": "c9", "model": "gpt-5.3-codex", "choices": [choice(
            json!({"tool_calls": [{"index": 0, "id": "t1", "type": "function",
                                   "function": {"name": "Bash", "arguments": "{}"}}]}), None)]}),
        json!({"id": "c9", "model": "gpt-5.3-codex",
               "choices": [choice(json!({}), Some("stop"))]}),
        json!({"id": "c9", "model": "gpt-5.3-codex", "choices": [],
               "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}}),
    ];

    let events = run(&chunks);
    let message_delta = events
        .iter()
        .find(|e| e["type"] == json!("message_delta"))
        .expect("message_delta emitted once usage arrives");
    assert_eq!(message_delta["delta"]["stop_reason"], json!("tool_use"));
    assert_eq!(
        events.last().expect("message_stop last")["type"],
        json!("message_stop")
    );
}