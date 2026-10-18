//! Port of `main/proxy/services/copilot/responses_translation.py` —
//! Anthropic Messages -> OpenAI Responses (non-streaming), plus the Responses
//! result -> Anthropic message reverse translation.
//!
//! Like the Python module, this reuses [`handle_system_prompt`] from
//! [`crate::non_stream_translation`] for the shared system-prompt helper.

use std::collections::BTreeMap;

use imc_proxy_types::types::anthropic::{
    AnthropicInputMessage, AnthropicMessagesPayload, AnthropicMetadata, AnthropicTextBlock,
    AnthropicToolChoice, AnthropicToolEntry, AnthropicToolResultContentBlock, StringOrBlocks,
};
use imc_proxy_types::types::responses::{
    FunctionTool, Reasoning, ResponseFunctionCallOutputItem, ResponseFunctionToolCallItem,
    ResponseInputContent, ResponseInputImage, ResponseInputMessage, ResponseInputText, ToolChoice,
    ToolChoiceFunction, ResponsesPayload,
};
use serde_json::{json, Value};

use crate::non_stream_translation::{
    assistant_content_block_view, handle_system_prompt, tool_result_block_text,
    user_content_block_view, ContentBlock,
};

/// Floor Copilot enforces on `max_output_tokens` for Responses requests.
const MIN_MAX_OUTPUT_TOKENS: i64 = 12800;

/// Port of `create_text_content`.
pub fn create_text_content(text: &str) -> ResponseInputText {
    ResponseInputText {
        r#type: "input_text".to_string(),
        text: text.to_string(),
    }
}

/// Port of `create_output_text_content`.
pub fn create_output_text_content(text: &str) -> ResponseInputText {
    ResponseInputText {
        r#type: "output_text".to_string(),
        text: text.to_string(),
    }
}

/// Port of `create_image_content` — a data-URL image with `detail: auto`.
pub fn create_image_content(media_type: &str, data: &str) -> ResponseInputImage {
    ResponseInputImage {
        r#type: "input_image".to_string(),
        image_url: Some(format!("data:{media_type};base64,{data}")),
        file_id: None,
        detail: "auto".to_string(),
    }
}

/// Port of `translate_user_content_block`.
///
/// Returns `[]` for block types the Responses API cannot carry (tool_result
/// and document are handled by the caller or dropped, as in Python).
pub fn translate_user_content_block(block: &ContentBlock) -> Vec<ResponseInputContent> {
    match block {
        ContentBlock::Text(text) => vec![ResponseInputContent::Text(create_text_content(text))],
        ContentBlock::Image { media_type, data } => vec![ResponseInputContent::Image(
            create_image_content(media_type, data),
        )],
        _ => Vec::new(),
    }
}

/// Port of `translate_assistant_content_block` — only `text` survives.
pub fn translate_assistant_content_block(block: &ContentBlock) -> Option<ResponseInputContent> {
    match block {
        ContentBlock::Text(text) => Some(ResponseInputContent::Text(create_output_text_content(
            text,
        ))),
        _ => None,
    }
}

/// Port of `translate_system_prompt`.
///
/// A string system prompt passes through untouched; a block list joins its
/// `text` blocks with a blank line and returns `None` when nothing text-like
/// is present.
pub fn translate_system_prompt(
    system: Option<&StringOrBlocks<AnthropicTextBlock>>,
) -> Option<String> {
    let system = system?;

    match system {
        StringOrBlocks::Text(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        StringOrBlocks::Blocks(blocks) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.r#type == "text")
                .map(|b| b.text.as_str())
                .collect();
            let text = parts.join("\n\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }
}

/// Port of `flush_pending_content` — drain buffered parts into one message.
///
/// `phase` is only set on assistant messages; user messages never carry one.
pub fn flush_pending_content(
    pending: &mut Vec<ResponseInputContent>,
    target: &mut Vec<Value>,
    role: &str,
    phase: Option<&str>,
) {
    if pending.is_empty() {
        return;
    }

    let msg = ResponseInputMessage {
        r#type: "message".to_string(),
        role: role.to_string(),
        content: Some(StringOrBlocks::Blocks(pending.clone())),
        status: None,
        phase: if role == "assistant" {
            phase.map(|p| p.to_string())
        } else {
            None
        },
    };

    target.push(to_json(&msg));
    pending.clear();
}

/// Port of `translate_message` — one Anthropic input message to Responses
/// input items.
///
/// Returning JSON values keeps the output item union (`message` /
/// `function_call` / `function_call_output`) faithful to the Python's
/// `List[Dict]` construction without pre-committing to one struct variant.
pub fn translate_message(message: &AnthropicInputMessage) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();

    match message {
        // Python checks `message.role == "system"` before dispatching, so a
        // system role wins over whichever untagged variant held it.
        AnthropicInputMessage::System(m) if m.role == "system" => {
            match &m.content {
                StringOrBlocks::Text(s) => {
                    items.push(to_json(&ResponseInputMessage {
                        r#type: "message".to_string(),
                        role: "developer".to_string(),
                        content: Some(StringOrBlocks::Text(s.clone())),
                        status: None,
                        phase: None,
                    }));
                }
                StringOrBlocks::Blocks(blocks) => {
                    let blocks: Vec<ResponseInputContent> = blocks
                        .iter()
                        .filter(|b| b.r#type == "text")
                        .map(|b| ResponseInputContent::Text(create_text_content(&b.text)))
                        .collect();
                    if !blocks.is_empty() {
                        items.push(to_json(&ResponseInputMessage {
                            r#type: "message".to_string(),
                            role: "developer".to_string(),
                            content: Some(StringOrBlocks::Blocks(blocks)),
                            status: None,
                            phase: None,
                        }));
                    }
                }
            }
            return items;
        }
        AnthropicInputMessage::User(m) if m.role == "user" => {
            match &m.content {
                StringOrBlocks::Text(s) => {
                    items.push(to_json(&ResponseInputMessage {
                        r#type: "message".to_string(),
                        role: "user".to_string(),
                        content: Some(StringOrBlocks::Text(s.clone())),
                        status: None,
                        phase: None,
                    }));
                }
                StringOrBlocks::Blocks(blocks) => {
                    let mut pending: Vec<ResponseInputContent> = Vec::new();
                    for block in blocks {
                        let view = user_content_block_view(block);
                        if view.type_str() == "tool_result" {
                            flush_pending_content(&mut pending, &mut items, "user", None);

                            let (call_id, content, is_error) = match &view {
                                ContentBlock::ToolResult {
                                    tool_use_id,
                                    content,
                                    is_error,
                                } => (tool_use_id.clone(), content, *is_error),
                                _ => continue,
                            };
                            let output = tool_result_block_text(content);

                            items.push(to_json(&ResponseFunctionCallOutputItem {
                                r#type: "function_call_output".to_string(),
                                call_id,
                                output: StringOrBlocks::Text(output),
                                status: Some(if is_error.unwrap_or(false) {
                                    "incomplete".to_string()
                                } else {
                                    "completed".to_string()
                                }),
                            }));
                        } else {
                            pending.extend(translate_user_content_block(&view));
                        }
                    }
                    flush_pending_content(&mut pending, &mut items, "user", None);
                }
            }
            return items;
        }
        AnthropicInputMessage::Assistant(m) if m.role == "assistant" => {
            match &m.content {
                StringOrBlocks::Text(s) => {
                    items.push(to_json(&ResponseInputMessage {
                        r#type: "message".to_string(),
                        role: "assistant".to_string(),
                        content: Some(StringOrBlocks::Text(s.clone())),
                        status: None,
                        phase: Some("final_answer".to_string()),
                    }));
                }
                StringOrBlocks::Blocks(blocks) => {
                    let views: Vec<ContentBlock> =
                        blocks.iter().map(assistant_content_block_view).collect();

                    let has_tool_use = views.iter().any(|b| b.type_str() == "tool_use");
                    let phase = if has_tool_use {
                        "commentary"
                    } else {
                        "final_answer"
                    };

                    let mut pending: Vec<ResponseInputContent> = Vec::new();
                    for view in &views {
                        match view {
                            ContentBlock::ToolUse { id, name, input } => {
                                flush_pending_content(&mut pending, &mut items, "assistant", Some(phase));

                                items.push(to_json(&ResponseFunctionToolCallItem {
                                    r#type: "function_call".to_string(),
                                    call_id: id.clone(),
                                    name: name.clone(),
                                    arguments: serde_json::to_string(input)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                    status: Some("completed".to_string()),
                                    namespace: None,
                                }));
                            }
                            // "ignore for now in translation as it's not well
                            // supported in inputs"
                            ContentBlock::Thinking { .. } => {}
                            other => {
                                if let Some(converted) = translate_assistant_content_block(other) {
                                    pending.push(converted);
                                }
                            }
                        }
                    }
                    flush_pending_content(&mut pending, &mut items, "assistant", Some(phase));
                }
            }
            return items;
        }
        _ => {}
    }

    items
}

/// Fields of a tool entry, from either `AnthropicTool` or the raw-dict form.
pub struct ResponsesToolFields {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<BTreeMap<String, Value>>,
}

/// Port of the field access `translate_responses_tools` performs per tool.
pub fn responses_tool_fields(entry: &AnthropicToolEntry) -> ResponsesToolFields {
    match entry {
        AnthropicToolEntry::Tool(t) => ResponsesToolFields {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        },
        AnthropicToolEntry::Raw(v) => ResponsesToolFields {
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: v
                .get("description")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            input_schema: v
                .get("input_schema")
                .and_then(Value::as_object)
                .map(|o| o.into_iter().collect::<BTreeMap<String, Value>>()),
        },
    }
}

/// Port of `translate_responses_tools`.
///
/// The Responses API takes `type`/`name`/`description`/`parameters` at the top
/// level rather than nested under `function`; `strict` is always emitted.
pub fn translate_responses_tools(
    tools: Option<&[AnthropicToolEntry]>,
) -> Option<Vec<FunctionTool>> {
    let tools = tools?;
    if tools.is_empty() {
        return None;
    }

    let responses_tools: Vec<FunctionTool> = tools
        .iter()
        .map(|t| {
            let fields = responses_tool_fields(t);
            FunctionTool {
                r#type: "function".to_string(),
                name: fields.name,
                description: fields.description,
                parameters: fields.input_schema,
                strict: Some(false),
            }
        })
        .collect();

    Some(responses_tools)
}

/// Port of `translate_responses_tools`'s `tool_choice` mapping.
pub fn translate_responses_tool_choice(
    tool_choice: Option<&AnthropicToolChoice>,
) -> Option<ToolChoice> {
    let tc = tool_choice?;
    match tc.r#type.as_str() {
        t @ ("auto" | "none") => Some(ToolChoice::Mode(t.to_string())),
        "any" => Some(ToolChoice::Mode("required".to_string())),
        "tool" => match tc.name.as_deref().filter(|n| !n.is_empty()) {
            Some(name) => Some(ToolChoice::Function(ToolChoiceFunction {
                r#type: "function".to_string(),
                name: name.to_string(),
            })),
            None => None,
        },
        _ => None,
    }
}

/// Port of the `metadata` mapping in `translate_to_responses`.
///
/// Python calls `payload.metadata.model_dump()`, which yields
/// `{"user_id": None}` when the id is unset; pydantic then rejects that for a
/// `Dict[str, str]` field, so the unset case is dropped here instead of
/// crashing. Everything else is copied verbatim.
pub fn translate_metadata(metadata: Option<&AnthropicMetadata>) -> Option<BTreeMap<String, String>> {
    let metadata = metadata?;
    let mut out = BTreeMap::new();
    if let Some(user_id) = metadata.user_id.as_deref().filter(|s| !s.is_empty()) {
        out.insert("user_id".to_string(), user_id.to_string());
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Port of `translate_to_responses` — an Anthropic Messages request translated
/// into a Responses request body.
pub fn translate_to_responses(payload: &AnthropicMessagesPayload) -> ResponsesPayload {
    let mut input_items: Vec<Value> = Vec::new();

    for msg in &payload.messages {
        input_items.extend(translate_message(msg));
    }

    let instructions = translate_system_prompt(payload.system.as_ref());
    let tools = translate_responses_tools(payload.tools.as_deref());
    let tool_choice = translate_responses_tool_choice(payload.tool_choice.as_ref());

    // Thinking forces temperature to 1.0 — Copilot rejects fractional
    // temperatures on reasoning models.
    let temperature = if payload.thinking.is_some() {
        Some(1.0)
    } else {
        payload.temperature
    };

    let max_output_tokens = match payload.max_tokens {
        0 => Some(MIN_MAX_OUTPUT_TOKENS),
        n => Some(n.max(MIN_MAX_OUTPUT_TOKENS)),
    };

    let reasoning = payload.thinking.as_ref().map(|_| Reasoning {
        effort: Some("max".to_string()),
        summary: Some("auto".to_string()),
        context: Some("auto".to_string()),
    });

    ResponsesPayload {
        model: payload.model.clone(),
        instructions,
        input: if input_items.is_empty() {
            None
        } else {
            Some(StringOrBlocks::Blocks(input_items))
        },
        tools: tools.map(|ts| ts.into_iter().map(|t| imc_proxy_types::types::responses::Tool::Function(t)).collect()),
        tool_choice,
        temperature,
        top_p: payload.top_p,
        max_output_tokens,
        metadata: translate_metadata(payload.metadata.as_ref()),
        stream: payload.stream,
        parallel_tool_calls: Some(true),
        store: Some(false),
        reasoning,
        include: None,
    }
}

/// Port of `translate_responses_non_stream_result` — a completed Responses
/// result to the Anthropic message format the client expects.
///
/// Kept as raw JSON in and out because Python builds the result dict by hand
/// rather than through a model.
pub fn translate_responses_non_stream_result(result: &Value) -> Value {
    let mut anthropic_content: Vec<Value> = Vec::new();

    let outputs = result.get("output").and_then(Value::as_array).cloned().unwrap_or_default();

    for output in &outputs {
        match output.get("type").and_then(Value::as_str) {
            Some("message") => {
                let content_blocks = output
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for block in &content_blocks {
                    if block.get("type").and_then(Value::as_str) == Some("output_text") {
                        anthropic_content.push(json!({
                            "type": "text",
                            "text": block.get("text").and_then(Value::as_str).unwrap_or(""),
                        }));
                    }
                }
            }
            Some("function_call") => {
                let args = output
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_string();
                let args_dict: Value = serde_json::from_str(&args).unwrap_or_else(|_| json!({}));

                anthropic_content.push(json!({
                    "type": "tool_use",
                    "id": output.get("call_id").cloned().unwrap_or(Value::Null),
                    "name": output.get("name").cloned().unwrap_or(Value::Null),
                    "input": args_dict,
                }));
            }
            _ => {}
        }
    }

    let usage = result.get("usage").cloned().unwrap_or(json!({}));
    let input_details = usage
        .get("input_tokens_details")
        .cloned()
        .unwrap_or(json!({}));

    json!({
        "id": result.get("id").cloned().unwrap_or(json!("msg_12345")),
        "type": "message",
        "role": "assistant",
        "content": anthropic_content,
        "model": result.get("model").cloned().unwrap_or(json!("gpt-5.3-codex")),
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "cache_creation_input_tokens": json_int(&input_details, "cache_write_tokens"),
            "cache_read_input_tokens": json_int(&input_details, "cached_tokens"),
            "input_tokens": json_int(&usage, "input_tokens"),
            "output_tokens": json_int(&usage, "output_tokens"),
        }
    })
}

/// Python's `d.get(key, 0)` for an integer usage counter.
fn json_int(obj: &Value, key: &str) -> i64 {
    obj.get(key).and_then(Value::as_i64).unwrap_or(0)
}