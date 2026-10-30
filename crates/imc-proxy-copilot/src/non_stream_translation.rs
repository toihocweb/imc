//! Port of `main/proxy/services/copilot/non_stream_translation.py` —
//! Anthropic Messages -> OpenAI Chat Completions (non-streaming).
//!
//! The Python module is the shared bottom layer of the proxy: it owns the
//! block-classification helpers (`map_content`, `handle_tool_result_block`, …)
//! that `responses_translation.py` imports back, so those live here too.
//!
//! ## Port notes
//!
//! * Python dispatches on `getattr(block, "type", None) == "..."` throughout.
//!   That is reproduced by [`ContentBlock`] + the `*_content_block_view`
//!   helpers, which re-tag a block by its `type` discriminator rather than
//!   trusting which untagged enum variant it landed in. The block structs in
//!   `imc-proxy-types` declare every field `#[serde(default)]`, so their
//!   untagged enums accept *any* JSON object into their first variant; keying
//!   off `r#type` is the faithful translation of the Python and the only
//!   variant-agnostic way to stay correct.
//! * Python `Optional[Union[str, List[Block]]]` is the shared `StringOrBlocks<T>`
//!   helper in `imc-proxy-types`.

use std::collections::BTreeMap;

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicAssistantMessage, AnthropicInputMessage,
    AnthropicMessagesPayload, AnthropicSystemMessage, AnthropicTextBlock, AnthropicToolChoice,
    AnthropicToolEntry, AnthropicToolResultContentBlock, AnthropicUserContentBlock,
    AnthropicUserMessage, StringOrBlocks,
};
use imc_proxy_types::types::chat_completions::{
    ChatCompletionsPayload, ContentPart, ImagePart, ImageUrl, Message, StringOrList, TextPart,
    Tool, ToolCall, ToolChoice, ToolChoiceFunction, ToolChoiceObj, ToolFunction, ToolFunctionDef,
};
use serde_json::{Map, Value};

/// Port of `THINKING_TEXT` — a placeholder thinking block that carries no
/// signal and is dropped before it reaches the OpenAI payload.
pub const THINKING_TEXT: &str = "Thinking...";

/// Port of `DEFAULT_EMPTY_CONTENT` — substituted wherever OpenAI rejects a
/// `null` content field.
pub const DEFAULT_EMPTY_CONTENT: &str = "follow imc-rule";

type AnyMap = BTreeMap<String, Value>;

/// Locally-defined block view.
///
/// `imc-proxy-types` models Anthropic content blocks as untagged enums, which
/// gives no reliable "which block is this" signal (see module docs). The
/// Python only ever asks for a block's `type` discriminator and then the
/// fields of the block that discriminator names, so this enum carries exactly
/// that much.
#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text(String),
    Image {
        media_type: String,
        data: String,
    },
    Document {
        media_type: String,
        data: String,
        title: Option<String>,
    },
    ToolReference(String),
    ToolUse {
        id: String,
        name: String,
        input: AnyMap,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolResult {
        tool_use_id: String,
        content: StringOrBlocks<AnthropicToolResultContentBlock>,
        is_error: Option<bool>,
    },
    /// A block whose `type` this translator does not handle.
    Unknown(String),
}

impl ContentBlock {
    /// The block's `type` discriminator — what Python's
    /// `getattr(block, "type", None)` returns.
    pub fn type_str(&self) -> &str {
        match self {
            ContentBlock::Text(_) => "text",
            ContentBlock::Image { .. } => "image",
            ContentBlock::Document { .. } => "document",
            ContentBlock::ToolReference(_) => "tool_reference",
            ContentBlock::ToolUse { .. } => "tool_use",
            ContentBlock::Thinking { .. } => "thinking",
            ContentBlock::ToolResult { .. } => "tool_result",
            ContentBlock::Unknown(t) => t,
        }
    }

    /// Python `block.text`, or `None` when the block has no `text` attribute.
    pub fn text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Python `block.thinking`, or `None` when the block has no `thinking`.
    pub fn thinking(&self) -> Option<&str> {
        match self {
            ContentBlock::Thinking { thinking, .. } => Some(thinking),
            _ => None,
        }
    }

    /// Python `block.signature` (only `thinking` blocks carry one).
    pub fn signature(&self) -> Option<&str> {
        match self {
            ContentBlock::Thinking { signature, .. } => Some(signature),
            _ => None,
        }
    }

    /// Python `block.tool_name` (only `tool_reference` blocks carry one).
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            ContentBlock::ToolReference(name) => Some(name),
            _ => None,
        }
    }
}

/// Re-tag a user content block by its `type` field.
pub fn user_content_block_view(block: &AnthropicUserContentBlock) -> ContentBlock {
    match block {
        AnthropicUserContentBlock::Text(b) if b.r#type == "text" => {
            ContentBlock::Text(b.text.clone())
        }
        AnthropicUserContentBlock::Image(b) if b.r#type == "image" => ContentBlock::Image {
            media_type: b.source.media_type.clone(),
            data: b.source.data.clone(),
        },
        AnthropicUserContentBlock::Document(b) if b.r#type == "document" => {
            ContentBlock::Document {
                media_type: b.source.media_type.clone(),
                data: b.source.data.clone(),
                title: b.title.clone(),
            }
        }
        AnthropicUserContentBlock::ToolResult(b) if b.r#type == "tool_result" => {
            ContentBlock::ToolResult {
                tool_use_id: b.tool_use_id.clone(),
                content: b.content.clone(),
                is_error: b.is_error,
            }
        }
        other => ContentBlock::Unknown(raw_user_type(other)),
    }
}

/// Re-tag an assistant content block by its `type` field.
pub fn assistant_content_block_view(block: &AnthropicAssistantContentBlock) -> ContentBlock {
    match block {
        AnthropicAssistantContentBlock::Text(b) if b.r#type == "text" => {
            ContentBlock::Text(b.text.clone())
        }
        AnthropicAssistantContentBlock::ToolUse(b) if b.r#type == "tool_use" => {
            ContentBlock::ToolUse {
                id: b.id.clone(),
                name: b.name.clone(),
                input: b.input.clone(),
            }
        }
        AnthropicAssistantContentBlock::Thinking(b) if b.r#type == "thinking" => {
            ContentBlock::Thinking {
                thinking: b.thinking.clone(),
                signature: b.signature.clone(),
            }
        }
        other => ContentBlock::Unknown(raw_assistant_type(other)),
    }
}

/// Re-tag a `tool_result` content block by its `type` field.
pub fn tool_result_content_block_view(block: &AnthropicToolResultContentBlock) -> ContentBlock {
    match block {
        AnthropicToolResultContentBlock::Text(b) if b.r#type == "text" => {
            ContentBlock::Text(b.text.clone())
        }
        AnthropicToolResultContentBlock::Image(b) if b.r#type == "image" => ContentBlock::Image {
            media_type: b.source.media_type.clone(),
            data: b.source.data.clone(),
        },
        AnthropicToolResultContentBlock::Document(b) if b.r#type == "document" => {
            ContentBlock::Document {
                media_type: b.source.media_type.clone(),
                data: b.source.data.clone(),
                title: b.title.clone(),
            }
        }
        AnthropicToolResultContentBlock::ToolReference(b) => {
            ContentBlock::ToolReference(b.tool_name.clone())
        }
        // The other arms carry guards, so Rust cannot prove exhaustiveness.
        other => ContentBlock::Unknown(raw_tool_result_type(other)),
    }
}

fn raw_tool_result_type(block: &AnthropicToolResultContentBlock) -> String {
    match block {
        AnthropicToolResultContentBlock::Text(b) => b.r#type.clone(),
        AnthropicToolResultContentBlock::Image(b) => b.r#type.clone(),
        AnthropicToolResultContentBlock::Document(b) => b.r#type.clone(),
        AnthropicToolResultContentBlock::ToolReference(b) => b.r#type.clone(),
    }
}

fn raw_user_type(block: &AnthropicUserContentBlock) -> String {
    match block {
        AnthropicUserContentBlock::Text(b) => b.r#type.clone(),
        AnthropicUserContentBlock::Image(b) => b.r#type.clone(),
        AnthropicUserContentBlock::Document(b) => b.r#type.clone(),
        AnthropicUserContentBlock::ToolResult(b) => b.r#type.clone(),
    }
}

fn raw_assistant_type(block: &AnthropicAssistantContentBlock) -> String {
    match block {
        AnthropicAssistantContentBlock::Text(b) => b.r#type.clone(),
        AnthropicAssistantContentBlock::ToolUse(b) => b.r#type.clone(),
        AnthropicAssistantContentBlock::Thinking(b) => b.r#type.clone(),
    }
}

/// Flatten any `StringOrBlocks` content into a uniform [`ContentBlock`] list.
///
/// A bare-string content becomes a single `text` block, which is what every
/// caller of `map_content` / `translate_message` wants.
pub fn flatten_content<T, F>(content: &StringOrBlocks<T>, view: F) -> Vec<ContentBlock>
where
    F: Fn(&T) -> ContentBlock,
{
    match content {
        StringOrBlocks::Text(s) => vec![ContentBlock::Text(s.clone())],
        StringOrBlocks::Blocks(blocks) => blocks.iter().map(view).collect(),
    }
}

/// Port of `handle_system_prompt`.
///
/// `system` is `Union[str, List[AnthropicTextBlock]]`; both forms collapse into
/// a single `system` message, and an empty prompt yields no message at all.
pub fn handle_system_prompt(system: Option<&StringOrBlocks<AnthropicTextBlock>>) -> Vec<Message> {
    let Some(system) = system else {
        return Vec::new();
    };

    match system {
        StringOrBlocks::Text(s) => {
            // `if not system: return []` — an empty string yields no message.
            if s.is_empty() {
                Vec::new()
            } else {
                vec![system_message(s.clone())]
            }
        }
        StringOrBlocks::Blocks(blocks) => {
            let parts: Vec<&str> = blocks.iter().map(|b| b.text.as_str()).collect();
            let system_text = join_non_empty(&parts, "\n\n");
            if system_text.is_empty() {
                Vec::new()
            } else {
                vec![system_message(system_text)]
            }
        }
    }
}

fn system_message(content: String) -> Message {
    Message {
        role: "system".to_string(),
        content: Some(StringOrBlocks::Text(content)),
        ..Message::default()
    }
}

/// Python's `"\n\n".join(p for p in parts if p)`.
pub fn join_non_empty(parts: &[&str], sep: &str) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<&str>>()
        .join(sep)
}

/// Port of `map_content` — Anthropic blocks to OpenAI `ContentPart`s.
///
/// Only `text`, `image` and `tool_reference` survive; every other block type
/// (thinking, tool_use, tool_result, document) contributes nothing. Returns
/// `None` when nothing mapped, matching Python's empty-list short circuit.
pub fn map_content(blocks: &[ContentBlock]) -> Option<Vec<ContentPart>> {
    let mut parts: Vec<ContentPart> = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text(t) => parts.push(ContentPart::Text(TextPart {
                r#type: "text".to_string(),
                text: t.clone(),
                cache_control: None,
            })),
            ContentBlock::Image { media_type, data } => {
                parts.push(ContentPart::Image(ImagePart {
                    r#type: "image_url".to_string(),
                    image_url: ImageUrl {
                        url: format!("data:{media_type};base64,{data}"),
                        detail: None,
                    },
                    cache_control: None,
                }))
            }
            ContentBlock::ToolReference(name) => parts.push(ContentPart::Text(TextPart {
                r#type: "text".to_string(),
                text: format!("Tool {name} loaded"),
                cache_control: None,
            })),
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// Python's `"\n".join([b.text for b in block.content if type == "text" and b.text])`
/// over a `tool_result`'s block-list content.
pub fn tool_result_block_text(content: &StringOrBlocks<AnthropicToolResultContentBlock>) -> String {
    match content {
        StringOrBlocks::Text(s) => s.clone(),
        StringOrBlocks::Blocks(blocks) => {
            let parts: Vec<String> = blocks
                .iter()
                .map(tool_result_content_block_view)
                .filter(|b| b.type_str() == "text")
                .filter_map(|b| b.text().map(|t| t.to_string()))
                .filter(|t| !t.is_empty())
                .collect();
            parts.join("\n")
        }
    }
}

/// Port of `handle_tool_result_block`.
pub fn handle_tool_result_block(block: &ContentBlock) -> Message {
    let (tool_use_id, content_text) = match block {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => (tool_use_id.clone(), tool_result_block_text(content)),
        _ => (String::new(), String::new()),
    };

    Message {
        role: "tool".to_string(),
        tool_call_id: Some(tool_use_id),
        content: Some(StringOrBlocks::Text(if content_text.is_empty() {
            DEFAULT_EMPTY_CONTENT.to_string()
        } else {
            content_text
        })),
        ..Message::default()
    }
}

/// Port of `handle_user_message`.
///
/// Tool results are emitted first, each as its own `tool` message, because
/// OpenAI requires them to directly follow the assistant's `tool_calls`; the
/// remaining blocks collapse into a single `user` message.
pub fn handle_user_message(message: &AnthropicUserMessage) -> Vec<Message> {
    let mut new_messages: Vec<Message> = Vec::new();

    match &message.content {
        StringOrBlocks::Text(s) => {
            new_messages.push(Message {
                role: "user".to_string(),
                content: Some(StringOrBlocks::Text(s.clone())),
                ..Message::default()
            });
        }
        StringOrBlocks::Blocks(blocks) => {
            let views: Vec<ContentBlock> = blocks.iter().map(user_content_block_view).collect();

            // Tool results MUST come directly after the assistant's tool_calls.
            for block in views.iter().filter(|b| b.type_str() == "tool_result") {
                new_messages.push(handle_tool_result_block(block));
            }

            let other_blocks: Vec<ContentBlock> = views
                .into_iter()
                .filter(|b| b.type_str() != "tool_result")
                .collect();

            if !other_blocks.is_empty() {
                if let Some(mapped) = map_content(&other_blocks) {
                    new_messages.push(Message {
                        role: "user".to_string(),
                        content: Some(StringOrBlocks::Blocks(mapped)),
                        ..Message::default()
                    });
                }
            }
        }
    }

    new_messages
}

/// Port of `handle_assistant_message`.
///
/// Thinking blocks never reach OpenAI as content; they are folded into the
/// non-standard `reasoning_text` / `reasoning_opaque` fields Copilot reads.
pub fn handle_assistant_message(message: &AnthropicAssistantMessage) -> Vec<Message> {
    let blocks = match &message.content {
        StringOrBlocks::Text(s) => {
            return vec![Message {
                role: "assistant".to_string(),
                content: Some(StringOrBlocks::Text(s.clone())),
                ..Message::default()
            }];
        }
        StringOrBlocks::Blocks(blocks) => {
            if blocks.is_empty() {
                return Vec::new();
            }
            blocks
        }
    };

    let views: Vec<ContentBlock> = blocks.iter().map(assistant_content_block_view).collect();

    let tool_use_blocks: Vec<&ContentBlock> =
        views.iter().filter(|b| b.type_str() == "tool_use").collect();

    let thinking_contents: Vec<String> = views
        .iter()
        .filter(|b| b.type_str() == "thinking")
        .filter_map(|b| b.thinking())
        .filter(|t| !t.is_empty() && *t != THINKING_TEXT)
        .map(|t| t.to_string())
        .collect();
    let all_thinking = if thinking_contents.is_empty() {
        None
    } else {
        Some(thinking_contents.join("\n\n"))
    };

    let signature = views
        .iter()
        .filter(|b| b.type_str() == "thinking")
        .find_map(|b| b.signature().filter(|s| !s.is_empty()))
        .map(|s| s.to_string());

    let content = match map_content(&views) {
        Some(parts) => StringOrBlocks::Blocks(parts),
        None => StringOrBlocks::Text(DEFAULT_EMPTY_CONTENT.to_string()),
    };

    if !tool_use_blocks.is_empty() {
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        for tu in tool_use_blocks {
            let ContentBlock::ToolUse { id, name, input } = tu else {
                continue;
            };
            // Python: `json.dumps(tu.input)`.
            let arguments = crate::python_json::python_json_dumps(
                &Value::Object(input.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            );
            tool_calls.push(ToolCall {
                id: id.clone(),
                r#type: "function".to_string(),
                function: ToolFunction {
                    name: name.clone(),
                    arguments,
                },
            });
        }

        return vec![Message {
            role: "assistant".to_string(),
            content: Some(content),
            tool_calls: Some(tool_calls),
            reasoning_text: all_thinking,
            reasoning_opaque: signature,
            ..Message::default()
        }];
    }

    vec![Message {
        role: "assistant".to_string(),
        content: Some(content),
        reasoning_text: all_thinking,
        reasoning_opaque: signature,
        ..Message::default()
    }]
}

/// Port of `translate_tools`.
///
/// The empty-schema fallback and the "object without properties" repair both
/// exist because Copilot rejects a tool whose parameters omit `properties`.
pub fn translate_tools(tools: Option<&[AnthropicToolEntry]>) -> Option<Vec<Tool>> {
    let tools = tools?;
    if tools.is_empty() {
        return None;
    }

    let mut out: Vec<Tool> = Vec::new();
    for entry in tools {
        let fields = tool_entry_fields(entry);
        let mut schema: Map<String, Value> = match &fields.input_schema {
            Some(schema) => schema.clone().into_iter().collect(),
            None => {
                let mut m = Map::new();
                m.insert("type".to_string(), Value::String("object".to_string()));
                m.insert("properties".to_string(), Value::Object(Map::new()));
                m
            }
        };
        if schema.get("type").and_then(Value::as_str) == Some("object")
            && !schema.contains_key("properties")
        {
            schema.insert("properties".to_string(), Value::Object(Map::new()));
        }

        out.push(Tool {
            r#type: "function".to_string(),
            function: ToolFunctionDef {
                name: fields.name,
                description: fields.description,
                parameters: schema.into_iter().collect(),
                strict: None,
            },
        });
    }
    Some(out)
}

/// Fields of an `AnthropicTool`, or of the raw-dict fallback the payload type
/// allows (`Union[AnthropicTool, Dict[str, Any]]`).
struct ToolFields {
    name: String,
    description: Option<String>,
    input_schema: Option<AnyMap>,
}

fn tool_entry_fields(entry: &AnthropicToolEntry) -> ToolFields {
    match entry {
        AnthropicToolEntry::Tool(t) => ToolFields {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        },
        AnthropicToolEntry::Raw(v) => ToolFields {
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
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        },
    }
}

/// Port of `translate_tool_choice`.
pub fn translate_tool_choice(tool_choice: Option<&AnthropicToolChoice>) -> Option<ToolChoice> {
    let tc = tool_choice?;
    match tc.r#type.as_str() {
        "auto" => Some(ToolChoice::Mode("auto".to_string())),
        "any" => Some(ToolChoice::Mode("required".to_string())),
        "tool" => match tc.name.as_deref().filter(|n| !n.is_empty()) {
            Some(name) => Some(ToolChoice::Object(ToolChoiceObj {
                r#type: "function".to_string(),
                function: ToolChoiceFunction {
                    name: name.to_string(),
                },
            })),
            // `tool` without a name falls through to the "none" default.
            None => Some(ToolChoice::Mode("none".to_string())),
        },
        _ => Some(ToolChoice::Mode("none".to_string())),
    }
}

pub fn handle_system_message(message: &AnthropicSystemMessage) -> Vec<Message> {
    match &message.content {
        StringOrBlocks::Text(s) => vec![Message {
            role: "system".to_string(),
            content: Some(StringOrBlocks::Text(s.clone())),
            ..Message::default()
        }],
        StringOrBlocks::Blocks(blocks) => {
            let text = blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            vec![Message {
                role: "system".to_string(),
                content: Some(StringOrBlocks::Text(text)),
                ..Message::default()
            }]
        }
    }
}

/// Port of `translate_to_openai` — an Anthropic Messages request translated
/// into a Chat Completions request body.
pub fn translate_to_openai(payload: &AnthropicMessagesPayload) -> ChatCompletionsPayload {
    let mut messages: Vec<Message> = Vec::new();
    messages.extend(handle_system_prompt(payload.system.as_ref()));

    for msg in &payload.messages {
        // Python branches on `msg.role`, not on the untagged enum variant.
        match msg {
            AnthropicInputMessage::User(m) if m.role == "user" => {
                messages.extend(handle_user_message(m));
            }
            AnthropicInputMessage::Assistant(m) if m.role == "assistant" => {
                messages.extend(handle_assistant_message(m));
            }
            AnthropicInputMessage::System(m) => {
                messages.extend(handle_system_message(m));
            }
            _ => {}
        }
    }

    // OpenAI does not support thinking_budget/reasoning_effort dynamically yet
    // for Copilot endpoints generally, but we map what we can.
    ChatCompletionsPayload {
        model: payload.model.clone(),
        messages,
        max_completion_tokens: Some(payload.max_tokens),
        stop: payload
            .stop_sequences
            .as_ref()
            .map(|s| StringOrList::List(s.clone())),
        stream: payload.stream,
        temperature: payload.temperature,
        top_p: payload.top_p,
        tools: translate_tools(payload.tools.as_deref()),
        tool_choice: translate_tool_choice(payload.tool_choice.as_ref()),
        ..Default::default()
    }
}