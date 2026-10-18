//! Port of `main/proxy/lib/types/responses.py` — OpenAI Responses API.
//!
//! Literal-typed fields are `String` (see `types/mod.rs`); Python `Union[...]`
//! becomes an untagged enum with the union's own name.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::anthropic::StringOrBlocks;

type AnyMap = BTreeMap<String, Value>;

/// Port of `FunctionTool`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FunctionTool {
    #[serde(rename = "type", default = "default_function")]
    pub r#type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<AnyMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

fn default_function() -> String {
    "function".to_string()
}

/// Port of the `Tool` Union (`FunctionTool | Dict[str, Any]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Tool {
    Function(FunctionTool),
    Raw(Value),
}

/// Port of `ResponseInputText` (`type` defaults to `input_text`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseInputText {
    /// Literal["input_text", "output_text"]
    #[serde(rename = "type", default = "default_input_text")]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
}

fn default_input_text() -> String {
    "input_text".to_string()
}

/// Port of `ResponseInputImage` (`detail` is one of low|high|auto).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseInputImage {
    #[serde(rename = "type", default = "default_input_image")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default)]
    pub detail: String,
}

fn default_input_image() -> String {
    "input_image".to_string()
}

/// Port of `ResponseInputFile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseInputFile {
    #[serde(rename = "type", default = "default_input_file")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

fn default_input_file() -> String {
    "input_file".to_string()
}

/// Port of the `ResponseInputContent` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseInputContent {
    Text(ResponseInputText),
    Image(ResponseInputImage),
    File(ResponseInputFile),
}

/// Port of `ResponseInputMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseInputMessage {
    #[serde(rename = "type", default = "default_message")]
    pub r#type: String,
    /// Literal["user", "assistant", "system", "developer"]
    #[serde(default)]
    pub role: String,
    /// Python: `Optional[Union[str, List[ResponseInputContent]]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<StringOrBlocks<ResponseInputContent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Literal["commentary", "final_answer"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

fn default_message() -> String {
    "message".to_string()
}

/// Port of `ResponseFunctionToolCallItem`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseFunctionToolCallItem {
    #[serde(rename = "type", default = "default_function_call")]
    pub r#type: String,
    #[serde(default)]
    pub call_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    /// Literal["in_progress", "completed", "incomplete"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

fn default_function_call() -> String {
    "function_call".to_string()
}

/// Port of `ResponseFunctionCallOutputItem`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseFunctionCallOutputItem {
    #[serde(rename = "type", default = "default_function_call_output")]
    pub r#type: String,
    #[serde(default)]
    pub call_id: String,
    /// Python: `Union[str, List[ResponseInputContent]]`.
    #[serde(default)]
    pub output: StringOrBlocks<ResponseInputContent>,
    /// Literal["in_progress", "completed", "incomplete"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

fn default_function_call_output() -> String {
    "function_call_output".to_string()
}

/// Port of the `ResponseInputItem` Union.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseInputItem {
    Message(ResponseInputMessage),
    FunctionCall(ResponseFunctionToolCallItem),
    FunctionCallOutput(ResponseFunctionCallOutputItem),
}

/// Port of `Reasoning`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Reasoning {
    /// Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Literal["auto", "concise", "detailed"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Literal["auto", "current_turn", "all_turns"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Port of `ToolChoiceFunction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolChoiceFunction {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default = "default_function")]
    pub r#type: String,
}

/// Port of `Union[Literal["none","auto","required"], ToolChoiceFunction]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Function(ToolChoiceFunction),
}

/// Port of `ResponsesPayload` — the `/v1/responses` request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesPayload {
    #[serde(default)]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Python: `Optional[Union[str, List[ResponseInputItem]]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<StringOrBlocks<ResponseInputItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
}

// --- Stream events --------------------------------------------------------

/// Port of `ResponseOutputText`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputText {
    #[serde(rename = "type", default = "default_output_text")]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub annotations: Vec<Value>,
}

fn default_output_text() -> String {
    "output_text".to_string()
}

/// Port of `ResponseOutputRefusal`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputRefusal {
    #[serde(rename = "type", default = "default_refusal")]
    pub r#type: String,
    #[serde(default)]
    pub refusal: String,
}

fn default_refusal() -> String {
    "refusal".to_string()
}

/// Port of `ResponseOutputContentBlock` Union (`... | Dict[str, Any]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseOutputContentBlock {
    Text(ResponseOutputText),
    Refusal(ResponseOutputRefusal),
    Raw(Value),
}

impl Default for ResponseOutputContentBlock {
    fn default() -> Self {
        ResponseOutputContentBlock::Text(ResponseOutputText::default())
    }
}

/// Port of `ResponseOutputMessage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputMessage {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "default_message")]
    pub r#type: String,
    #[serde(default)]
    pub role: String,
    /// Literal["completed", "in_progress", "incomplete"]
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ResponseOutputContentBlock>>,
}

/// Port of `ResponseOutputFunctionCall`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputFunctionCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default = "default_function_call")]
    pub r#type: String,
    #[serde(default)]
    pub call_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
    /// Literal["in_progress", "completed", "incomplete"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Port of the `ResponseOutputItem` Union (`... | Dict[str, Any]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseOutputItem {
    Message(ResponseOutputMessage),
    FunctionCall(ResponseOutputFunctionCall),
    Raw(Value),
}

impl Default for ResponseOutputItem {
    fn default() -> Self {
        ResponseOutputItem::Message(ResponseOutputMessage::default())
    }
}

/// Port of `ResponsesResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesResult {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "object", default = "default_response")]
    pub object: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub output: Vec<ResponseOutputItem>,
    #[serde(default)]
    pub output_text: String,
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AnyMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_usage: Option<AnyMap>,
}

fn default_response() -> String {
    "response".to_string()
}

/// Port of `ResponseCompletedEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseCompletedEvent {
    #[serde(default)]
    pub response: ResponsesResult,
    #[serde(default)]
    pub sequence_number: i64,
    /// Literal["response.completed"]
    #[serde(rename = "type", default = "default_response_completed")]
    pub r#type: String,
}

fn default_response_completed() -> String {
    "response.completed".to_string()
}

/// Port of `ResponseErrorEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseErrorEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_error")]
    pub r#type: String,
}

fn default_error() -> String {
    "error".to_string()
}

/// Port of `ResponseFunctionCallArgumentsDeltaEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseFunctionCallArgumentsDeltaEvent {
    #[serde(default)]
    pub delta: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_fca_delta")]
    pub r#type: String,
}

fn default_fca_delta() -> String {
    "response.function_call_arguments.delta".to_string()
}

/// Port of `ResponseFunctionCallArgumentsDoneEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseFunctionCallArgumentsDoneEvent {
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_fca_done")]
    pub r#type: String,
}

fn default_fca_done() -> String {
    "response.function_call_arguments.done".to_string()
}

/// Port of `ResponseContentPartAddedEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseContentPartAddedEvent {
    #[serde(default)]
    pub content_index: i64,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub part: ResponseOutputContentBlock,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_cp_added")]
    pub r#type: String,
}

fn default_cp_added() -> String {
    "response.content_part.added".to_string()
}

/// Port of `ResponseContentPartDoneEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseContentPartDoneEvent {
    #[serde(default)]
    pub content_index: i64,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub part: ResponseOutputContentBlock,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_cp_done")]
    pub r#type: String,
}

fn default_cp_done() -> String {
    "response.content_part.done".to_string()
}

/// Port of `ResponseTextDeltaEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseTextDeltaEvent {
    #[serde(default)]
    pub content_index: i64,
    #[serde(default)]
    pub delta: String,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_text_delta")]
    pub r#type: String,
}

fn default_text_delta() -> String {
    "response.output_text.delta".to_string()
}

/// Port of `ResponseTextDoneEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseTextDoneEvent {
    #[serde(default)]
    pub content_index: i64,
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(default)]
    pub text: String,
    #[serde(rename = "type", default = "default_text_done")]
    pub r#type: String,
}

fn default_text_done() -> String {
    "response.output_text.done".to_string()
}

/// Port of `ResponseOutputItemAddedEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputItemAddedEvent {
    #[serde(default)]
    pub item: ResponseOutputItem,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_item_added")]
    pub r#type: String,
}

fn default_item_added() -> String {
    "response.output_item.added".to_string()
}

/// Port of `ResponseOutputItemDoneEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseOutputItemDoneEvent {
    #[serde(default)]
    pub item: ResponseOutputItem,
    #[serde(default)]
    pub output_index: i64,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_item_done")]
    pub r#type: String,
}

fn default_item_done() -> String {
    "response.output_item.done".to_string()
}

/// Port of `ResponseCreatedEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseCreatedEvent {
    #[serde(default)]
    pub response: ResponsesResult,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_created")]
    pub r#type: String,
}

fn default_created() -> String {
    "response.created".to_string()
}

/// Port of `ResponseInProgressEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponseInProgressEvent {
    #[serde(default)]
    pub response: ResponsesResult,
    #[serde(default)]
    pub sequence_number: i64,
    #[serde(rename = "type", default = "default_in_progress")]
    pub r#type: String,
}

fn default_in_progress() -> String {
    "response.in_progress".to_string()
}

/// Port of `ResponsePingEvent`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsePingEvent {
    #[serde(rename = "type", default = "default_ping")]
    pub r#type: String,
}

fn default_ping() -> String {
    "ping".to_string()
}

/// Port of the `ResponseStreamEvent` Union — untagged; `type` discriminates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseStreamEvent {
    Completed(ResponseCompletedEvent),
    Error(ResponseErrorEvent),
    FunctionCallArgumentsDelta(ResponseFunctionCallArgumentsDeltaEvent),
    FunctionCallArgumentsDone(ResponseFunctionCallArgumentsDoneEvent),
    ContentPartAdded(ResponseContentPartAddedEvent),
    ContentPartDone(ResponseContentPartDoneEvent),
    TextDelta(ResponseTextDeltaEvent),
    TextDone(ResponseTextDoneEvent),
    OutputItemAdded(ResponseOutputItemAddedEvent),
    OutputItemDone(ResponseOutputItemDoneEvent),
    Created(ResponseCreatedEvent),
    InProgress(ResponseInProgressEvent),
    Ping(ResponsePingEvent),
}