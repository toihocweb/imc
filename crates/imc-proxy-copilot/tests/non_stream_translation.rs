//! Non-stream translation round trip: a realistic Anthropic request ->
//! Responses payload, then a realistic Responses result -> Anthropic message.
//!
//! The payload is built with typed constructors rather than
//! `serde_json::from_value`, because the `imc-proxy-types` untagged block enums
//! accept any object into their first variant (every field is
//! `#[serde(default)]`), which loses `tool_use`/`tool_result`/`image` payloads
//! on the way in. See the crate report for that upstream issue.

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicAssistantMessage, AnthropicImageBlock,
    AnthropicImageSource, AnthropicInputMessage, AnthropicMetadata, AnthropicMessagesPayload,
    AnthropicTextBlock, AnthropicThinkingBlock, AnthropicThinkingConfig, AnthropicTool,
    AnthropicToolChoice, AnthropicToolEntry, AnthropicToolResultBlock,
    AnthropicToolResultContentBlock, AnthropicUserContentBlock, AnthropicUserMessage,
    StringOrBlocks,
};
use serde_json::{json, Value};

use imc_proxy_copilot::responses_translation::{
    translate_responses_non_stream_result, translate_to_responses,
};

fn tool_use(id: &str, name: &str, command: &str) -> AnthropicAssistantContentBlock {
    let mut input = std::collections::BTreeMap::new();
    input.insert("command".to_string(), Value::String(command.to_string()));
    AnthropicAssistantContentBlock::ToolUse(imc_proxy_types::types::anthropic::AnthropicToolUseBlock {
        r#type: "tool_use".to_string(),
        id: id.to_string(),
        name: name.to_string(),
        input,
        cache_control: None,
    })
}

/// A realistic multi-turn request: system prompt, a user turn carrying an
/// image plus a tool result, an assistant turn that thought, answered and
/// called a tool, then the follow-up user turn.
fn realistic_payload() -> AnthropicMessagesPayload {
    let mut tool_choice = AnthropicToolChoice::default();
    tool_choice.r#type = "any".to_string();

    let mut tool = AnthropicTool::default();
    tool.name = "Bash".to_string();
    tool.description = Some("Run a shell command".to_string());
    let mut schema = std::collections::BTreeMap::new();
    schema.insert(
        "type".to_string(),
        Value::String("object".to_string()),
    );
    let mut props = serde_json::Map::new();
    props.insert(
        "command".to_string(),
        json!({"type": "string", "description": "The command to run"}),
    );
    schema.insert("properties".to_string(), Value::Object(props));
    tool.input_schema = Some(schema);

    let mut metadata = AnthropicMetadata::default();
    metadata.user_id = Some("user_42".to_string());

    AnthropicMessagesPayload {
        model: "gpt-5.3-codex".to_string(),
        messages: vec![
            // A user turn with text, an image, and a tool result.
            AnthropicInputMessage::User(AnthropicUserMessage {
                role: "user".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicUserContentBlock::Text(AnthropicTextBlock {
                        r#type: "text".to_string(),
                        text: "What does this screenshot show?".to_string(),
                        cache_control: None,
                    }),
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
                            AnthropicToolResultContentBlock::Text(AnthropicTextBlock {
                                r#type: "text".to_string(),
                                text: "total 12\ndrwxr-xr-x 2 root root 4096 .".to_string(),
                                cache_control: None,
                            }),
                        ]),
                        is_error: None,
                        cache_control: None,
                    }),
                ]),
            }),
            // An assistant turn that thought, then answered and called a tool.
            AnthropicInputMessage::Assistant(AnthropicAssistantMessage {
                role: "assistant".to_string(),
                content: StringOrBlocks::Blocks(vec![
                    AnthropicAssistantContentBlock::Thinking(AnthropicThinkingBlock {
                        r#type: "thinking".to_string(),
                        thinking: "The user wants the listing plus the screenshot.".to_string(),
                        signature: "sig_abc123".to_string(),
                    }),
                    AnthropicAssistantContentBlock::Text(AnthropicTextBlock {
                        r#type: "text".to_string(),
                        text: "Let me check the directory.".to_string(),
                        cache_control: None,
                    }),
                    tool_use("toolu_02", "Bash", "ls -la"),
                ]),
            }),
        ],
        system: Some(StringOrBlocks::Blocks(vec![AnthropicTextBlock {
            r#type: "text".to_string(),
            text: "You are a terse coding assistant.".to_string(),
            cache_control: None,
        }])),
        stop_sequences: Some(vec!["STOP".to_string()]),
        stream: Some(false),
        top_p: Some(0.9),
        top_k: Some(40),
        tools: Some(vec![AnthropicToolEntry::Tool(tool)]),
        tool_choice: Some(tool_choice),
        max_tokens: 4096,
        thinking: Some(AnthropicThinkingConfig {
            r#type: "enabled".to_string(),
            budget_tokens: Some(1024),
            display: None,
        }),
        service_tier: None,
        output_config: None,
        metadata: Some(metadata),
        temperature: Some(0.2),
        cache_control: None,
    }
}

/// Serialize the way the Python route does: `model_dump(exclude_none=True)`.
fn dump(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serialize")
}

#[test]
fn anthropic_request_translates_to_responses_payload() {
    let payload = realistic_payload();
    let responses = translate_to_responses(&payload);
    let body = dump(&responses);

    // System prompt becomes top-level instructions, not an input message.
    assert_eq!(
        body["instructions"],
        json!("You are a terse coding assistant."),
        "system prompt should map to instructions"
    );
    assert!(
        body["input"]
            .as_array()
            .expect("input is a list")
            .iter()
            .all(|item| item["role"] != json!("system")),
        "no inline system message should appear in input"
    );

    // Turn 1: text + image message, then the tool result.
    let input = body["input"].as_array().expect("input is a list");
    assert_eq!(input.len(), 4, "text/image, tool output, assistant, tool call");

    assert_eq!(input[0]["type"], json!("message"));
    assert_eq!(input[0]["role"], json!("user"));
    assert_eq!(input[0]["content"][0]["type"], json!("input_text"));
    assert_eq!(
        input[0]["content"][0]["text"],
        json!("What does this screenshot show?")
    );
    assert_eq!(input[0]["content"][1]["type"], json!("input_image"));
    assert_eq!(input[0]["content"][1]["detail"], json!("auto"));
    assert_eq!(
        input[0]["content"][1]["image_url"],
        json!("data:image/png;base64,iVBORw0KGgo=")
    );

    // Tool result -> function_call_output, with the tool_use_id as call_id.
    assert_eq!(input[1]["type"], json!("function_call_output"));
    assert_eq!(input[1]["call_id"], json!("toolu_01"));
    assert_eq!(input[1]["status"], json!("completed"));
    assert_eq!(
        input[1]["output"],
        json!("total 12\ndrwxr-xr-x 2 root root 4096 .")
    );

    // Turn 2: the assistant's text is an output_text message in "commentary"
    // phase (it carries a tool call), and the tool_use becomes a function_call.
    assert_eq!(input[2]["type"], json!("message"));
    assert_eq!(input[2]["role"], json!("assistant"));
    assert_eq!(input[2]["phase"], json!("commentary"));
    assert_eq!(input[2]["content"][0]["type"], json!("output_text"));
    assert_eq!(
        input[2]["content"][0]["text"],
        json!("Let me check the directory.")
    );

    assert_eq!(input[3]["type"], json!("function_call"));
    assert_eq!(input[3]["call_id"], json!("toolu_02"));
    assert_eq!(input[3]["name"], json!("Bash"));
    assert_eq!(input[3]["status"], json!("completed"));
    // Arguments are serialized with Python's `json.dumps` defaults: `", "` and
    // `": "` separators.
    assert_eq!(input[3]["arguments"], json!(r#"{"command": "ls -la"}"#));

    // Tools are flattened to the Responses top-level shape with strict: false.
    let tool = &body["tools"][0];
    assert_eq!(tool["type"], json!("function"));
    assert_eq!(tool["name"], json!("Bash"));
    assert_eq!(tool["description"], json!("Run a shell command"));
    assert_eq!(tool["strict"], json!(false));
    assert_eq!(tool["parameters"]["type"], json!("object"));
    assert!(tool["parameters"]["properties"]["command"].is_object());

    // `any` tool choice maps to "required".
    assert_eq!(body["tool_choice"], json!("required"));

    // Thinking forces temperature to 1.0 and raises the reasoning config.
    assert_eq!(body["temperature"], json!(1.0));
    assert_eq!(body["reasoning"]["effort"], json!("max"));
    assert_eq!(body["reasoning"]["summary"], json!("auto"));
    assert_eq!(body["reasoning"]["context"], json!("auto"));

    // max_tokens 4096 is below the 12800 floor, so it is raised.
    assert_eq!(body["max_output_tokens"], json!(12800));
    assert_eq!(body["top_p"], json!(0.9));
    assert_eq!(body["metadata"]["user_id"], json!("user_42"));
    assert_eq!(body["parallel_tool_calls"], json!(true));
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["stream"], json!(false));

    // `exclude_none` semantics: absent fields are omitted entirely.
    assert!(body.get("include").is_none() || body["include"].is_null());
    assert!(body["top_k"].is_null() || body.get("top_k").is_none());
}

#[test]
fn responses_result_translates_back_to_anthropic_message() {
    let result = json!({
        "id": "resp_abc",
        "object": "response",
        "created_at": 1730000000,
        "model": "gpt-5.3-codex",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {"type": "output_text", "text": "It shows a directory listing."},
                    {"type": "refusal", "refusal": "no"}
                ]
            },
            {
                "type": "function_call",
                "call_id": "call_789",
                "name": "Bash",
                "arguments": "{\"command\":\"pwd\"}",
                "status": "completed"
            }
        ],
        "usage": {
            "input_tokens": 120,
            "output_tokens": 34,
            "input_tokens_details": {"cached_tokens": 20, "cache_write_tokens": 5}
        }
    });

    let out = translate_responses_non_stream_result(&result);

    assert_eq!(out["id"], json!("resp_abc"));
    assert_eq!(out["type"], json!("message"));
    assert_eq!(out["role"], json!("assistant"));
    assert_eq!(out["model"], json!("gpt-5.3-codex"));
    assert_eq!(out["stop_reason"], json!("end_turn"));
    assert_eq!(out["stop_sequence"], json!(null));

    let content = out["content"].as_array().expect("content is a list");
    assert_eq!(content.len(), 2, "only output_text and function_call survive");
    assert_eq!(content[0]["type"], json!("text"));
    assert_eq!(content[0]["text"], json!("It shows a directory listing."));
    assert_eq!(content[1]["type"], json!("tool_use"));
    assert_eq!(content[1]["id"], json!("call_789"));
    assert_eq!(content[1]["name"], json!("Bash"));
    assert_eq!(content[1]["input"], json!({"command": "pwd"}));

    assert_eq!(out["usage"]["input_tokens"], json!(120));
    assert_eq!(out["usage"]["output_tokens"], json!(34));
    assert_eq!(out["usage"]["cache_read_input_tokens"], json!(20));
    assert_eq!(out["usage"]["cache_creation_input_tokens"], json!(5));
}

#[test]
fn responses_result_with_unparsable_arguments_defaults_to_empty_object() {
    // `id` and `model` are deliberately absent: Python's `result.get(key,
    // default)` only falls back when the key is missing.
    let result = json!({
        "output": [
            {"type": "function_call", "call_id": "c1", "name": "Bash", "arguments": "{not json"},
            {"type": "message", "content": [{"type": "output_text", "text": "ok"}]}
        ]
    });

    let out = translate_responses_non_stream_result(&result);
    let content = out["content"].as_array().expect("content is a list");
    assert_eq!(content[0]["input"], json!({}), "bad arguments become an empty object");
    // Missing usage falls back to zeroes rather than null.
    assert_eq!(out["usage"]["input_tokens"], json!(0));
    assert_eq!(out["usage"]["cache_read_input_tokens"], json!(0));
    // Missing id/model fall back to the Python literals.
    assert_eq!(out["id"], json!("msg_12345"));
    assert_eq!(out["model"], json!("gpt-5.3-codex"));
}

#[test]
fn empty_tool_choice_tool_type_is_dropped() {
    let mut payload = realistic_payload();
    let mut tc = AnthropicToolChoice::default();
    tc.r#type = "tool".to_string();
    tc.name = None;
    payload.tool_choice = Some(tc);

    let body = dump(&translate_to_responses(&payload));
    assert!(
        body["tool_choice"].is_null() || body.get("tool_choice").is_none(),
        "a tool choice naming no tool produces no tool_choice"
    );
}

#[test]
fn no_thinking_keeps_fractional_temperature_and_never_adds_reasoning() {
    let mut payload = realistic_payload();
    payload.thinking = None;

    let body = dump(&translate_to_responses(&payload));
    assert_eq!(body["temperature"], json!(0.2));
    assert!(
        body["reasoning"].is_null() || body.get("reasoning").is_none(),
        "no thinking config -> no reasoning block"
    );
}