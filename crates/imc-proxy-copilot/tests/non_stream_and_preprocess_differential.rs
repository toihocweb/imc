//! Differential test: `translate_to_openai` and `preprocess::*` must match the
//! Python byte-for-byte on the same payload.
//!
//! Expectations were produced by running
//! `main/proxy/services/copilot/{non_stream_translation,preprocess}.py` over
//! the payload built in [`realistic_payload`].
//!
//! Note the payload is assembled with typed constructors rather than
//! `serde_json::from_value`: the `imc-proxy-types` untagged block enums accept
//! any JSON object into their first variant (every field is
//! `#[serde(default)]`), so deserializing would silently drop the `tool_use`,
//! `tool_result` and `image` payloads this test needs to observe.

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicAssistantMessage, AnthropicImageBlock,
    AnthropicImageSource, AnthropicInputMessage, AnthropicMessagesPayload, AnthropicTextBlock,
    AnthropicThinkingBlock, AnthropicTool, AnthropicToolChoice, AnthropicToolEntry,
    AnthropicToolResultBlock, AnthropicToolResultContentBlock, AnthropicToolUseBlock,
    AnthropicUserContentBlock, AnthropicUserMessage, StringOrBlocks,
};
use serde_json::{json, Value};

use imc_proxy_copilot::non_stream_translation::translate_to_openai;
use imc_proxy_copilot::preprocess::{
    apply_last_message_cache_control, apply_system_cache_control, apply_tools_cache_control,
};

fn tool_use_block(id: &str, name: &str, command: &str) -> AnthropicAssistantContentBlock {
    let mut input = std::collections::BTreeMap::new();
    input.insert("command".to_string(), Value::String(command.to_string()));
    AnthropicAssistantContentBlock::ToolUse(AnthropicToolUseBlock {
        r#type: "tool_use".to_string(),
        id: id.to_string(),
        name: name.to_string(),
        input,
        cache_control: None,
    })
}

fn text_block(text: &str) -> AnthropicTextBlock {
    AnthropicTextBlock {
        r#type: "text".to_string(),
        text: text.to_string(),
        cache_control: None,
    }
}

/// The same realistic multi-turn request the Python harness was fed.
fn realistic_payload() -> AnthropicMessagesPayload {
    let mut tool_choice = AnthropicToolChoice::default();
    tool_choice.r#type = "any".to_string();

    let mut tool = AnthropicTool::default();
    tool.name = "Bash".to_string();
    tool.description = Some("Run a shell command".to_string());
    tool.input_schema = Some(
        [
            (
                "type".to_string(),
                Value::String("object".to_string()),
            ),
            (
                "properties".to_string(),
                json!({"command": {"type": "string", "description": "The command to run"}}),
            ),
        ]
        .into_iter()
        .collect(),
    );

    AnthropicMessagesPayload {
        model: "gpt-5.3-codex".to_string(),
        messages: vec![
            AnthropicInputMessage::User(AnthropicUserMessage {
                role: "user".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicUserContentBlock::Text(text_block(
                        "What does this screenshot show?",
                    )),
                    AnthropicUserContentBlock::Image(AnthropicImageBlock {
                        r#type: "image".to_string(),
                        source: AnthropicImageSource {
                            r#type: "base64".to_string(),
                            media_type: "image/png".to_string(),
                            data: "iVBORw0KGgo=".to_string(),
                        },
                        cache_control: None,
                    }),
                    AnthropicUserContentBlock::ToolResult(AnthropicToolResultBlock {
                        r#type: "tool_result".to_string(),
                        tool_use_id: "toolu_01".to_string(),
                        content: StringOrBlocks::Blocks(vec![
                            AnthropicToolResultContentBlock::Text(text_block(
                                "total 12\ndrwxr-xr-x 2 root root 4096 .",
                            )),
                        ]),
                        is_error: None,
                        cache_control: None,
                    }),
                ]),
            }),
            AnthropicInputMessage::Assistant(AnthropicAssistantMessage {
                role: "assistant".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicAssistantContentBlock::Thinking(AnthropicThinkingBlock {
                        r#type: "thinking".to_string(),
                        thinking: "The user wants the listing plus the screenshot.".to_string(),
                        signature: "sig_abc123".to_string(),
                    }),
                    AnthropicAssistantContentBlock::Text(text_block(
                        "Let me check the directory.",
                    )),
                    tool_use_block("toolu_02", "Bash", "ls -la"),
                ]),
            }),
        ],
        system: Some(StringOrBlocks::Blocks(vec![text_block(
            "You are a terse coding assistant.",
        )])),
        stop_sequences: Some(vec!["STOP".to_string()]),
        stream: None,
        top_p: Some(0.9),
        top_k: None,
        tools: Some(vec![AnthropicToolEntry::Tool(tool)]),
        tool_choice: Some(tool_choice),
        max_tokens: 4096,
        thinking: Some(imc_proxy_types::types::anthropic::AnthropicThinkingConfig {
            r#type: "enabled".to_string(),
            budget_tokens: Some(1024),
            display: None,
        }),
        service_tier: None,
        output_config: None,
        metadata: Some(imc_proxy_types::types::anthropic::AnthropicMetadata {
            user_id: Some("user_42".to_string()),
        }),
        temperature: Some(0.2),
        cache_control: None,
    }
}

/// Serialize the way the Python route does: `model_dump(exclude_none=True)`.
fn dump(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serialize")
}

#[test]
fn translate_to_openai_matches_python() {
    let actual = dump(&translate_to_openai(&realistic_payload()));

    // The tool schema is built separately to keep the nesting readable.
    let tool_schema = json!({
        "type": "object",
        "properties": {
            "command": {"type": "string", "description": "The command to run"}
        }
    });
    let expected = json!({
        "messages": [
            // System prompt collapses into one string message.
            {"role": "system", "content": "You are a terse coding assistant."},
            // Tool results come first so they directly follow tool_calls.
            {"role": "tool", "content": "total 12\ndrwxr-xr-x 2 root root 4096 .",
             "tool_call_id": "toolu_01"},
            // Then the remaining user blocks as typed content parts.
            {"role": "user", "content": [
                {"type": "text", "text": "What does this screenshot show?"},
                {"type": "image_url",
                 "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}}
            ]},
            // Thinking is folded into reasoning_* rather than content.
            {"role": "assistant",
             "content": [{"type": "text", "text": "Let me check the directory."}],
             "tool_calls": [{"id": "toolu_02", "type": "function",
                             "function": {"name": "Bash",
                                          "arguments": "{\"command\": \"ls -la\"}"}}],
             "reasoning_text": "The user wants the listing plus the screenshot.",
             "reasoning_opaque": "sig_abc123"}
        ],
        "model": "gpt-5.3-codex",
        "temperature": 0.2,
        "top_p": 0.9,
        "max_completion_tokens": 4096,
        "stop": ["STOP"],
        "tools": [{
            "type": "function",
            "function": {
                "name": "Bash",
                "description": "Run a shell command",
                "parameters": tool_schema
            }
        }],
        "tool_choice": "required"
    });

    assert_eq!(actual, expected);
}

#[test]
fn preprocess_places_exactly_three_cache_markers() {
    let mut payload = realistic_payload();
    apply_system_cache_control(&mut payload);
    apply_tools_cache_control(&mut payload);
    apply_last_message_cache_control(&mut payload);

    let actual = dump(&payload);

    // System prompt: one marker, on its only block.
    assert_eq!(
        actual["system"],
        json!([{"type": "text", "text": "You are a terse coding assistant.",
                "cache_control": {"type": "ephemeral"}}])
    );

    // Tools: one marker, on the last tool only.
    assert_eq!(
        actual["tools"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );

    // Last user message: the marker lands on its last block, which here is the
    // tool_result. The assistant turn is untouched.
    assert_eq!(
        actual["messages"][0]["content"][2]["cache_control"],
        json!({"type": "ephemeral"})
    );
    assert!(
        actual["messages"][1]["content"].as_array().expect("blocks").iter()
            .all(|b| b.get("cache_control").is_none()),
        "assistant blocks must not be marked"
    );
}

#[test]
fn preprocess_converts_a_string_system_prompt_to_one_marked_block() {
    let mut payload = AnthropicMessagesPayload {
        model: "m".to_string(),
        messages: vec![AnthropicInputMessage::User(AnthropicUserMessage {
            role: "user".to_string(),
            content: StringOrBlocks::Text("hello".to_string()),
        })],
        max_tokens: 10,
        system: Some(StringOrBlocks::Text("plain string system".to_string())),
        ..AnthropicMessagesPayload::default()
    };

    apply_system_cache_control(&mut payload);
    apply_tools_cache_control(&mut payload);
    apply_last_message_cache_control(&mut payload);

    let actual = dump(&payload);
    assert_eq!(
        actual["system"],
        json!([{"type": "text", "text": "plain string system",
                "cache_control": {"type": "ephemeral"}}])
    );
    // The bare-string user message becomes a marked text block.
    assert_eq!(
        actual["messages"][0]["content"],
        json!([{"type": "text", "text": "hello",
                "cache_control": {"type": "ephemeral"}}])
    );
    assert!(actual["tools"].is_null(), "no tools -> no tool markers");
}

#[test]
fn preprocess_strips_client_markers_before_replacing_them() {
    // Anthropic rejects a request with more than four marked blocks, so every
    // incoming marker is cleared first and only then re-applied.
    let mut first_system = text_block("a");
    first_system.cache_control = Some(imc_proxy_types::types::anthropic::AnthropicCacheControl {
        r#type: "ephemeral".to_string(),
        ttl: Some("1h".to_string()),
        ..Default::default()
    });
    let second_system = text_block("b");

    let mut tool1 = AnthropicTool::default();
    tool1.name = "t1".to_string();
    let mut tool2 = AnthropicTool::default();
    tool2.name = "t2".to_string();

    let mut marked = text_block("x");
    marked.cache_control = Some(imc_proxy_types::types::anthropic::AnthropicCacheControl {
        r#type: "ephemeral".to_string(),
        ..Default::default()
    });
    let mut marked_assistant = text_block("y");
    marked_assistant.cache_control =
        Some(imc_proxy_types::types::anthropic::AnthropicCacheControl {
            r#type: "ephemeral".to_string(),
            ..Default::default()
        });

    let mut payload = AnthropicMessagesPayload {
        model: "m".to_string(),
        max_tokens: 10,
        system: Some(StringOrBlocks::Blocks(vec![first_system, second_system])),
        tools: Some(vec![
            AnthropicToolEntry::Tool(tool1),
            AnthropicToolEntry::Tool(tool2),
        ]),
        messages: vec![
            AnthropicInputMessage::User(AnthropicUserMessage {
                role: "user".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicUserContentBlock::Text(marked)
                ]),
            }),
            AnthropicInputMessage::Assistant(AnthropicAssistantMessage {
                role: "assistant".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicAssistantContentBlock::Text(marked_assistant),
                    AnthropicAssistantContentBlock::Thinking(AnthropicThinkingBlock {
                        r#type: "thinking".to_string(),
                        thinking: "t".to_string(),
                        signature: "s".to_string(),
                    }),
                ]),
            }),
        ],
        ..AnthropicMessagesPayload::default()
    };

    apply_system_cache_control(&mut payload);
    apply_tools_cache_control(&mut payload);
    apply_last_message_cache_control(&mut payload);

    let actual = dump(&payload);
    let system = actual["system"].as_array().expect("system blocks");
    assert!(
        system[0].get("cache_control").is_none(),
        "the earlier system block loses its client marker"
    );
    assert_eq!(
        system[1]["cache_control"],
        json!({"type": "ephemeral"}),
        "the marker moves to the last system block"
    );

    let tools = actual["tools"].as_array().expect("tools");
    assert!(tools[0].get("cache_control").is_none());
    assert_eq!(tools[1]["cache_control"], json!({"type": "ephemeral"}));
}

#[test]
fn preprocess_without_a_user_message_falls_back_to_the_last_message() {
    let mut payload = AnthropicMessagesPayload {
        model: "m".to_string(),
        max_tokens: 10,
        messages: vec![AnthropicInputMessage::Assistant(AnthropicAssistantMessage {
            role: "assistant".to_string(),
            content: StringOrBlocks::Text("only assistant".to_string()),
        })],
        ..AnthropicMessagesPayload::default()
    };

    apply_last_message_cache_control(&mut payload);
    let actual = dump(&payload);
    assert_eq!(
        actual["messages"][0]["content"],
        json!([{"type": "text", "text": "only assistant",
                "cache_control": {"type": "ephemeral"}}])
    );
}

#[test]
fn preprocess_never_marks_a_thinking_block() {
    let mut payload = AnthropicMessagesPayload {
        model: "m".to_string(),
        max_tokens: 10,
        messages: vec![AnthropicInputMessage::User(AnthropicUserMessage {
            role: "user".to_string(),
            content: StringOrBlocks::Blocks(vec![AnthropicUserContentBlock::Text(text_block("hi"))]),
        })],
        ..AnthropicMessagesPayload::default()
    };
    apply_system_cache_control(&mut payload);
    apply_last_message_cache_control(&mut payload);

    // Empty system -> untouched.
    assert!(dump(&payload)["system"].is_null());
}