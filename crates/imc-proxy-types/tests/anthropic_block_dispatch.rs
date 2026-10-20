//! Ingress deserialization of Anthropic content blocks.
//!
//! These unions were `#[serde(untagged)]`, with a text variant first whose
//! every field carried `#[serde(default)]`. Untagged matching tries variants in
//! order and accepts the first that does not error, so *every* block — including
//! `tool_use`, `tool_result` and `image` — deserialized as an empty `Text`.
//! Nothing failed; the tool name, input and id were simply gone, and the proxy
//! would have forwarded the call stripped of everything that matters.

use imc_proxy_types::types::anthropic::*;
use serde_json::json;

fn payload(content: serde_json::Value) -> AnthropicMessagesPayload {
    serde_json::from_value(json!({
        "model": "claude-pro-max",
        "messages": [{ "role": "assistant", "content": content }]
    }))
    .expect("payload should deserialize")
}

#[test]
fn assistant_tool_use_keeps_its_payload() {
    let p = payload(json!([
        {"type": "text", "text": "let me check"},
        {"type": "tool_use", "id": "tu_1", "name": "imc_search",
         "input": {"query": "explain imc search"}}
    ]));
    let AnthropicInputMessage::Assistant(a) = &p.messages[0] else {
        panic!("expected assistant message")
    };
    let StringOrBlocks::Blocks(blocks) = &a.content else {
        panic!("expected block content, got text-only")
    };
    assert_eq!(blocks.len(), 2, "{blocks:?}");

    match &blocks[1] {
        AnthropicAssistantContentBlock::ToolUse(b) => {
            assert_eq!(b.id, "tu_1");
            assert_eq!(b.name, "imc_search");
            assert_eq!(b.input["query"], json!("explain imc search"));
        }
        other => panic!("tool_use landed in the wrong variant: {other:?}"),
    }
}

#[test]
fn user_tool_result_keeps_its_payload() {
    let p: AnthropicMessagesPayload = serde_json::from_value(json!({
        "model": "m",
        "messages": [{"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "tu_1", "content": "result text"}
        ]}]
    }))
    .unwrap();
    let AnthropicInputMessage::User(u) = &p.messages[0] else {
        panic!("expected user message")
    };
    let StringOrBlocks::Blocks(blocks) = &u.content else {
        panic!("expected block content")
    };
    match &blocks[0] {
        AnthropicUserContentBlock::ToolResult(b) => {
            assert_eq!(b.tool_use_id, "tu_1");
        }
        other => panic!("tool_result landed in the wrong variant: {other:?}"),
    }
}

#[test]
fn user_image_is_not_text() {
    let p: AnthropicMessagesPayload = serde_json::from_value(json!({
        "model": "m",
        "messages": [{"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAA"}}
        ]}]
    }))
    .unwrap();
    let AnthropicInputMessage::User(u) = &p.messages[0] else {
        panic!("expected user message")
    };
    let StringOrBlocks::Blocks(blocks) = &u.content else {
        panic!("expected block content")
    };
    assert!(
        matches!(blocks[0], AnthropicUserContentBlock::Image(_)),
        "image deserialized as {:?}",
        blocks[0]
    );
}

#[test]
fn assistant_thinking_is_not_text() {
    let p = payload(json!([
        {"type": "thinking", "thinking": "hmm", "signature": "sig"}
    ]));
    let AnthropicInputMessage::Assistant(a) = &p.messages[0] else {
        panic!("expected assistant")
    };
    let StringOrBlocks::Blocks(blocks) = &a.content else {
        panic!("expected block content")
    };
    assert!(
        matches!(blocks[0], AnthropicAssistantContentBlock::Thinking(_)),
        "thinking deserialized as {:?}",
        blocks[0]
    );
}

#[test]
fn text_block_still_works_and_keeps_cache_control() {
    let p = payload(json!([{"type": "text", "text": "hello"}]));
    let AnthropicInputMessage::Assistant(a) = &p.messages[0] else {
        panic!("expected assistant")
    };
    let StringOrBlocks::Blocks(blocks) = &a.content else {
        panic!("expected block content")
    };
    match &blocks[0] {
        AnthropicAssistantContentBlock::Text(b) => {
            assert_eq!(b.text, "hello");
            assert_eq!(b.r#type, "text");
        }
        other => panic!("text landed in the wrong variant: {other:?}"),
    }
}

#[test]
fn an_unknown_block_type_is_an_error_not_a_silent_text() {
    // Better to reject loudly than to hand the translator an empty text block.
    let out: Result<AnthropicMessagesPayload, _> = serde_json::from_value(json!({
        "model": "m",
        "messages": [{"role": "assistant", "content": [
            {"type": "server_tool_use", "id": "x", "name": "web_search"}
        ]}]
    }));
    assert!(
        out.is_err(),
        "unknown block type must not silently become a text block"
    );
}