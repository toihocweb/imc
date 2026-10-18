//! Port of `main/proxy/lib/types/chat_completions.py` — OpenAI Chat Completions.
//!
//! Literal-typed fields are `String` (see `types/mod.rs`); Python `Union[...]`
//! becomes an untagged enum with the union's own name.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::anthropic::StringOrBlocks;

type AnyMap = BTreeMap<String, Value>;

// --- Base types -----------------------------------------------------------

/// Port of `ToolFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolFunction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

/// Port of `ToolCall` (`type` defaults to `function`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "default_function")]
    pub r#type: String,
    #[serde(default)]
    pub function: ToolFunction,
}

fn default_function() -> String {
    "function".to_string()
}

/// Port of `CopilotUsage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopilotUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_nano_aiu: Option<f64>,
}

/// Port of `UsageDetails`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<i64>,
}

/// Port of `CompletionTokensDetails`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionTokensDetails {
    #[serde(default)]
    pub accepted_prediction_tokens: i64,
    #[serde(default)]
    pub rejected_prediction_tokens: i64,
}

/// Port of `Usage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_hit_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_miss_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens_details: Option<UsageDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

// --- Streaming types ------------------------------------------------------

/// Port of `DeltaFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Port of `DeltaToolCall` (`index` defaults to 0).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaToolCall {
    #[serde(default)]
    pub index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<DeltaFunction>,
}

/// Port of `Delta`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Delta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Literal["user", "assistant", "system", "tool"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_opaque: Option<String>,
}

/// Port of `Choice` (`index` defaults to 0).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub delta: Delta,
    /// Literal["stop", "length", "tool_calls", "content_filter"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<AnyMap>,
}

/// Port of `ChatCompletionChunk`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<CopilotUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

// --- Non-streaming types --------------------------------------------------

/// Port of `ResponseMessage` (`role` is always `assistant`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseMessage {
    #[serde(default = "default_assistant")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_opaque: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

fn default_assistant() -> String {
    "assistant".to_string()
}

/// Port of `ChoiceNonStreaming`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChoiceNonStreaming {
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub message: ResponseMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<AnyMap>,
    /// Literal["stop", "length", "tool_calls", "content_filter"]
    #[serde(default = "default_stop")]
    pub finish_reason: String,
}

fn default_stop() -> String {
    "stop".to_string()
}

/// Port of `ChatCompletionResponse` (`object` is always `chat.completion`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "object", default = "default_chat_completion")]
    pub object: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub choices: Vec<ChoiceNonStreaming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<CopilotUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

fn default_chat_completion() -> String {
    "chat.completion".to_string()
}

// --- Payload types --------------------------------------------------------

/// Port of `CacheControl` (`type` is always `ephemeral`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheControl {
    #[serde(rename = "type", default = "default_ephemeral")]
    pub r#type: String,
}

fn default_ephemeral() -> String {
    "ephemeral".to_string()
}

/// Port of `CopilotCacheControl`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopilotCacheControl {
    #[serde(rename = "type", default = "default_ephemeral")]
    pub r#type: String,
}

/// Port of `TextPart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(rename = "type", default = "default_text")]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

fn default_text() -> String {
    "text".to_string()
}

/// Port of `ImageUrl` (`detail` is one of low|high|auto).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageUrl {
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Port of `ImagePart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImagePart {
    #[serde(rename = "type", default = "default_image_url")]
    pub r#type: String,
    #[serde(default)]
    pub image_url: ImageUrl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

fn default_image_url() -> String {
    "image_url".to_string()
}

/// Port of `FileData`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileData {
    #[serde(default)]
    pub file_data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// Port of `FilePart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilePart {
    #[serde(rename = "type", default = "default_file")]
    pub r#type: String,
    #[serde(default)]
    pub file: FileData,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

fn default_file() -> String {
    "file".to_string()
}

/// Port of the `ContentPart` Union (`TextPart | ImagePart | FilePart`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentPart {
    Text(TextPart),
    Image(ImagePart),
    File(FilePart),
}

/// Port of `Message`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Message {
    /// Literal["user", "assistant", "system", "tool", "developer"]
    #[serde(default)]
    pub role: String,
    /// Python: `Optional[Union[str, List[ContentPart]]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<StringOrBlocks<ContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_opaque: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_cache_control: Option<CopilotCacheControl>,
}

/// Port of `ToolFunctionDef`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolFunctionDef {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: AnyMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

/// Port of `Tool` (`type` is always `function`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type", default = "default_function")]
    pub r#type: String,
    #[serde(default)]
    pub function: ToolFunctionDef,
}

/// Port of `StreamOptions`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

/// Port of `ResponseFormat` (`type` is always `json_object`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseFormat {
    #[serde(rename = "type", default = "default_json_object")]
    pub r#type: String,
}

fn default_json_object() -> String {
    "json_object".to_string()
}

/// Port of `ToolChoiceFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    #[serde(default)]
    pub name: String,
}

/// Port of `ToolChoiceObj`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolChoiceObj {
    #[serde(rename = "type", default = "default_function")]
    pub r#type: String,
    #[serde(default)]
    pub function: ToolChoiceFunction,
}

/// Port of `Union[Literal["none","auto","required"], ToolChoiceObj]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Object(ToolChoiceObj),
}

/// Port of `ChatCompletionsPayload` — the `/v1/chat/completions` request body.
/// Python sets `extra = "allow"`, hence the flattened passthrough map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatCompletionsPayload {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<StringOrList>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// `extra = "allow"`
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Port of Python's `Union[str, List[str]]` for `stop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    Text(String),
    List(Vec<String>),
}

impl StringOrList {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            StringOrList::Text(s) => Some(s),
            StringOrList::List(_) => None,
        }
    }

    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            StringOrList::List(l) => Some(l),
            StringOrList::Text(_) => None,
        }
    }
}