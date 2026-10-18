//! Port of `main/proxy/lib/types/anthropic.py` — Anthropic Messages protocol.
//!
//! Literal-typed fields are `String` here with the allowed values in the doc
//! comment; see `types/mod.rs` for why. Python `Union[...]` aliases become
//! untagged enums of the same name, and the union discriminators are ordered so
//! the specific variant is tried before the catch-all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

type AnyMap = BTreeMap<String, Value>;

/// Port of `BetaJSONOutputFormat` (`type` is always `json_schema`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BetaJSONOutputFormat {
    /// Python declares `schema_` with `alias="schema"`; accept both spellings.
    #[serde(
        default,
        rename = "schema",
        alias = "schema_",
        skip_serializing_if = "Option::is_none"
    )]
    pub schema_: Option<AnyMap>,
    /// Literal["json_schema"]
    #[serde(rename = "type", default = "default_json_schema")]
    pub r#type: String,
    /// `extra = "allow"`
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_json_schema() -> String {
    "json_schema".to_string()
}

/// Port of `AnthropicCacheControl` (`type` is always `ephemeral`;
/// `ttl` is one of `5m` / `1h`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicCacheControl {
    #[serde(rename = "type", default = "default_ephemeral")]
    pub r#type: String,
    /// Literal["5m", "1h"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// `extra = "allow"`
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_ephemeral() -> String {
    "ephemeral".to_string()
}

/// Port of `AnthropicTextBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicTextBlock {
    #[serde(rename = "type", default = "default_text")]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_text() -> String {
    "text".to_string()
}

/// Port of `AnthropicImageSource` (`media_type` is one of image/jpeg|png|gif|webp).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicImageSource {
    #[serde(rename = "type", default = "default_base64")]
    pub r#type: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub data: String,
}

fn default_base64() -> String {
    "base64".to_string()
}

/// Port of `AnthropicImageBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicImageBlock {
    #[serde(rename = "type", default = "default_image")]
    pub r#type: String,
    #[serde(default)]
    pub source: AnthropicImageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_image() -> String {
    "image".to_string()
}

/// Port of `AnthropicDocumentSource` (`media_type` is application/pdf).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicDocumentSource {
    #[serde(rename = "type", default = "default_base64")]
    pub r#type: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub data: String,
}

/// Port of `AnthropicDocumentBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicDocumentBlock {
    #[serde(rename = "type", default = "default_document")]
    pub r#type: String,
    #[serde(default)]
    pub source: AnthropicDocumentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_document() -> String {
    "document".to_string()
}

/// Port of `AnthropicToolReferenceBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicToolReferenceBlock {
    #[serde(rename = "type", default = "default_tool_reference")]
    pub r#type: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_tool_reference() -> String {
    "tool_reference".to_string()
}

/// Port of the `AnthropicToolResultContentBlock` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicToolResultContentBlock {
    Text(AnthropicTextBlock),
    Image(AnthropicImageBlock),
    Document(AnthropicDocumentBlock),
    ToolReference(AnthropicToolReferenceBlock),
}

/// Port of `AnthropicToolResultBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicToolResultBlock {
    #[serde(rename = "type", default = "default_tool_result")]
    pub r#type: String,
    #[serde(default)]
    pub tool_use_id: String,
    /// Python: `Union[str, List[AnthropicToolResultContentBlock]]` — modelled as
    /// an untagged pair so both the bare-string and block-list forms round-trip.
    #[serde(default)]
    pub content: StringOrBlocks<AnthropicToolResultContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_tool_result() -> String {
    "tool_result".to_string()
}

/// Port of `AnthropicToolUseBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicToolUseBlock {
    #[serde(rename = "type", default = "default_tool_use")]
    pub r#type: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: AnyMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
}

fn default_tool_use() -> String {
    "tool_use".to_string()
}

/// Port of `AnthropicThinkingBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicThinkingBlock {
    #[serde(rename = "type", default = "default_thinking")]
    pub r#type: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub signature: String,
}

fn default_thinking() -> String {
    "thinking".to_string()
}

/// Port of the `AnthropicUserContentBlock` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicUserContentBlock {
    Text(AnthropicTextBlock),
    Image(AnthropicImageBlock),
    Document(AnthropicDocumentBlock),
    ToolResult(AnthropicToolResultBlock),
}

/// Port of the `AnthropicAssistantContentBlock` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicAssistantContentBlock {
    Text(AnthropicTextBlock),
    ToolUse(AnthropicToolUseBlock),
    Thinking(AnthropicThinkingBlock),
}

/// Port of `AnthropicUserMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicUserMessage {
    /// Literal["user"]
    #[serde(default = "default_user_role")]
    pub role: String,
    /// Python's `normalize_null_content` validator maps `None` -> `""`.
    #[serde(default)]
    pub content: StringOrBlocks<AnthropicUserContentBlock>,
}

fn default_user_role() -> String {
    "user".to_string()
}

/// Port of `AnthropicAssistantMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicAssistantMessage {
    /// Literal["assistant"]
    #[serde(default = "default_assistant_role")]
    pub role: String,
    #[serde(default)]
    pub content: StringOrBlocks<AnthropicAssistantContentBlock>,
}

fn default_assistant_role() -> String {
    "assistant".to_string()
}

/// Port of `AnthropicSystemMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicSystemMessage {
    /// Literal["system"]
    #[serde(default = "default_system_role")]
    pub role: String,
    #[serde(default)]
    pub content: StringOrBlocks<AnthropicTextBlock>,
}

fn default_system_role() -> String {
    "system".to_string()
}

/// Port of the `AnthropicMessage` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicMessage {
    User(AnthropicUserMessage),
    Assistant(AnthropicAssistantMessage),
}

/// Port of the `AnthropicInputMessage` Union (`AnthropicMessage | AnthropicSystemMessage`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicInputMessage {
    User(AnthropicUserMessage),
    Assistant(AnthropicAssistantMessage),
    System(AnthropicSystemMessage),
}

/// Port of `AnthropicTool`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicTool {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<AnyMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defer_loading: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_domains: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_domains: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<AnyMap>,
    /// `extra = "allow"`
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Port of the `Union[AnthropicTool, Dict[str, Any]]` used by
/// `AnthropicMessagesPayload.tools`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicToolEntry {
    Tool(AnthropicTool),
    Raw(Value),
}

/// Port of `AnthropicWebSearchResultItem`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicWebSearchResultItem {
    #[serde(rename = "type", default = "default_web_search_result")]
    pub r#type: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_age: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

fn default_web_search_result() -> String {
    "web_search_result".to_string()
}

/// Port of `AnthropicServerToolUseBlock` (`name` is always `web_search`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicServerToolUseBlock {
    #[serde(rename = "type", default = "default_server_tool_use")]
    pub r#type: String,
    #[serde(default)]
    pub id: String,
    #[serde(default = "default_web_search")]
    pub name: String,
    #[serde(default)]
    pub input: AnyMap,
}

fn default_server_tool_use() -> String {
    "server_tool_use".to_string()
}
fn default_web_search() -> String {
    "web_search".to_string()
}

/// Port of `AnthropicWebSearchToolResultErrorBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicWebSearchToolResultErrorBlock {
    #[serde(rename = "type", default = "default_web_search_tool_result_error")]
    pub r#type: String,
    #[serde(default)]
    pub error_code: String,
}

fn default_web_search_tool_result_error() -> String {
    "web_search_tool_result_error".to_string()
}

/// Port of `AnthropicWebSearchToolResultBlock`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicWebSearchToolResultBlock {
    #[serde(rename = "type", default = "default_web_search_tool_result")]
    pub r#type: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: SearchResultOrError,
}

/// Port of `Union[List[AnthropicWebSearchResultItem], AnthropicWebSearchToolResultErrorBlock]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchResultOrError {
    Items(Vec<AnthropicWebSearchResultItem>),
    Error(AnthropicWebSearchToolResultErrorBlock),
}

impl Default for SearchResultOrError {
    fn default() -> Self {
        SearchResultOrError::Items(Vec::new())
    }
}

fn default_web_search_tool_result() -> String {
    "web_search_tool_result".to_string()
}

/// Port of the `AnthropicWebSearchContentBlock` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicWebSearchContentBlock {
    ServerToolUse(AnthropicServerToolUseBlock),
    ToolResult(AnthropicWebSearchToolResultBlock),
}

/// Port of `AnthropicUsage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
    /// Literal["standard", "priority", "batch"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<AnyMap>,
}

/// Port of `CopilotUsage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CopilotUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_nano_aiu: Option<f64>,
}

/// Port of the `AnthropicResponseContentBlock` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicResponseContentBlock {
    Text(AnthropicTextBlock),
    ToolUse(AnthropicToolUseBlock),
    Thinking(AnthropicThinkingBlock),
    ServerToolUse(AnthropicServerToolUseBlock),
    WebSearchToolResult(AnthropicWebSearchToolResultBlock),
}

/// Port of `AnthropicResponse`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicResponse {
    #[serde(default)]
    pub id: String,
    /// Literal["message"]
    #[serde(rename = "type", default = "default_message")]
    pub r#type: String,
    /// Literal["assistant"]
    #[serde(default = "default_assistant_role")]
    pub role: String,
    #[serde(default)]
    pub content: Vec<AnthropicResponseContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<CopilotUsage>,
    #[serde(default)]
    pub model: String,
    /// Literal["end_turn", "max_tokens", "stop_sequence", "tool_use", "pause_turn", "refusal"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    #[serde(default)]
    pub usage: AnthropicUsage,
}

fn default_message() -> String {
    "message".to_string()
}

// --- Stream events -------------------------------------------------------

/// Port of `AnthropicMessageStartEventMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageStartEventMessage {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "default_message")]
    pub r#type: String,
    #[serde(default = "default_assistant_role")]
    pub role: String,
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<CopilotUsage>,
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<Value>,
    #[serde(default)]
    pub usage: AnthropicUsage,
}

/// Port of `AnthropicMessageStartEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageStartEvent {
    #[serde(rename = "type", default = "default_message_start")]
    pub r#type: String,
    #[serde(default)]
    pub message: AnthropicMessageStartEventMessage,
}

fn default_message_start() -> String {
    "message_start".to_string()
}

/// Port of `AnthropicContentBlockStartEvent` (`content_block` is `Any` in Python).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicContentBlockStartEvent {
    #[serde(rename = "type", default = "default_content_block_start")]
    pub r#type: String,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub content_block: Value,
}

fn default_content_block_start() -> String {
    "content_block_start".to_string()
}

/// Port of `AnthropicContentBlockDeltaEvent` (`delta` is `Any` in Python).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicContentBlockDeltaEvent {
    #[serde(rename = "type", default = "default_content_block_delta")]
    pub r#type: String,
    #[serde(default)]
    pub index: i64,
    #[serde(default)]
    pub delta: Value,
}

fn default_content_block_delta() -> String {
    "content_block_delta".to_string()
}

/// Port of `AnthropicContentBlockStopEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicContentBlockStopEvent {
    #[serde(rename = "type", default = "default_content_block_stop")]
    pub r#type: String,
    #[serde(default)]
    pub index: i64,
}

fn default_content_block_stop() -> String {
    "content_block_stop".to_string()
}

/// Port of `AnthropicMessageDeltaEventDelta`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageDeltaEventDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
}

/// Port of `AnthropicMessageDeltaEventUsage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageDeltaEventUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<i64>,
}

/// Port of `AnthropicMessageDeltaEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageDeltaEvent {
    #[serde(rename = "type", default = "default_message_delta")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<CopilotUsage>,
    #[serde(default)]
    pub delta: AnthropicMessageDeltaEventDelta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AnthropicMessageDeltaEventUsage>,
}

fn default_message_delta() -> String {
    "message_delta".to_string()
}

/// Port of `AnthropicMessageStopEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessageStopEvent {
    #[serde(rename = "type", default = "default_message_stop")]
    pub r#type: String,
}

fn default_message_stop() -> String {
    "message_stop".to_string()
}

/// Port of `AnthropicPingEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicPingEvent {
    #[serde(rename = "type", default = "default_ping")]
    pub r#type: String,
}

fn default_ping() -> String {
    "ping".to_string()
}

/// Port of `AnthropicErrorEventError`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicErrorEventError {
    #[serde(rename = "type", default)]
    pub r#type: String,
    #[serde(default)]
    pub message: String,
}

/// Port of `AnthropicErrorEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicErrorEvent {
    #[serde(rename = "type", default = "default_error")]
    pub r#type: String,
    #[serde(default)]
    pub error: AnthropicErrorEventError,
}

fn default_error() -> String {
    "error".to_string()
}

// --- Stream state --------------------------------------------------------

/// Port of `AnthropicStreamStateToolCall` — note the camelCase
/// `anthropicBlockIndex` key is preserved on the wire.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicStreamStateToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "anthropicBlockIndex", default)]
    pub anthropic_block_index: i64,
}

/// Port of `AnthropicStreamState` — same camelCase wire keys as Python.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicStreamState {
    #[serde(rename = "messageStartSent", default)]
    pub message_start_sent: bool,
    #[serde(rename = "contentBlockIndex", default)]
    pub content_block_index: i64,
    #[serde(rename = "contentBlockOpen", default)]
    pub content_block_open: bool,
    #[serde(rename = "thinkingBlockOpen", default)]
    pub thinking_block_open: bool,
    #[serde(
        rename = "pendingMessageDelta",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub pending_message_delta: Option<AnthropicMessageDeltaEvent>,
    #[serde(
        rename = "deferredContent",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub deferred_content: Option<String>,
    #[serde(rename = "toolCalls", default)]
    pub tool_calls: BTreeMap<i64, AnthropicStreamStateToolCall>,
}

/// Port of `AnthropicToolChoice` (`type` is one of auto|any|tool|none).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicToolChoice {
    #[serde(rename = "type", default = "default_auto")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_auto() -> String {
    "auto".to_string()
}

/// Port of `AnthropicThinkingConfig` (`type` is enabled|adaptive).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicThinkingConfig {
    #[serde(rename = "type", default = "default_enabled")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

fn default_enabled() -> String {
    "enabled".to_string()
}

/// Port of `AnthropicOutputConfig` (`effort` is one of low|medium|high|xhigh|max).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicOutputConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<BetaJSONOutputFormat>,
}

/// Port of `AnthropicMetadata`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

/// Port of `AnthropicMessagesPayload` — the `/v1/messages` request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicMessagesPayload {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub messages: Vec<AnthropicInputMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<AnthropicCacheControl>,
    /// Python: `Union[str, List[AnthropicTextBlock]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<StringOrBlocks<AnthropicTextBlock>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<AnthropicToolEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(default)]
    pub max_tokens: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<AnthropicThinkingConfig>,
    /// Literal["auto", "standard_only"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_config: Option<AnthropicOutputConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<AnthropicMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

/// Port of the `AnthropicStreamEvent` set — untagged, matching the Python
/// module's event family. `type` is the discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnthropicStreamEvent {
    MessageStart(AnthropicMessageStartEvent),
    ContentBlockStart(AnthropicContentBlockStartEvent),
    ContentBlockDelta(AnthropicContentBlockDeltaEvent),
    ContentBlockStop(AnthropicContentBlockStopEvent),
    MessageDelta(AnthropicMessageDeltaEvent),
    MessageStop(AnthropicMessageStopEvent),
    Ping(AnthropicPingEvent),
    Error(AnthropicErrorEvent),
}

// ---------------------------------------------------------------------------
// Shared serde helper for Python's `Union[str, List[Block]]`
// ---------------------------------------------------------------------------

/// Rust stand-in for Python's `Union[str, List[T]]` content field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrBlocks<T> {
    Text(String),
    Blocks(Vec<T>),
}

impl<T> Default for StringOrBlocks<T> {
    fn default() -> Self {
        StringOrBlocks::Text(String::new())
    }
}

impl<T> StringOrBlocks<T> {
    /// Text payload when this variant holds a bare string.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            StringOrBlocks::Text(s) => Some(s),
            StringOrBlocks::Blocks(_) => None,
        }
    }

    /// Blocks when this variant holds a list.
    pub fn as_blocks(&self) -> Option<&[T]> {
        match self {
            StringOrBlocks::Blocks(b) => Some(b),
            StringOrBlocks::Text(_) => None,
        }
    }

    /// Python's `normalize_null_content`: `None` -> `""`.
    pub fn normalize_null_content() -> Self {
        StringOrBlocks::Text(String::new())
    }
}