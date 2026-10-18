//! Port of `main/proxy/lib/types/models.py` — the `/v1/models` response shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Port of `ModelLimitsVision`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelLimitsVision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_prompt_image_size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_prompt_images: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_media_types: Option<Vec<String>>,
}

/// Port of `ModelLimits`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_window_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_prompt_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inputs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<ModelLimitsVision>,
}

/// Port of `ModelSupports`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelSupports {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_thinking_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_thinking_budget: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_outputs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<Vec<String>>,
}

/// Port of `ModelCapabilities`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub family: String,
    pub limits: ModelLimits,
    pub object: String,
    pub supports: ModelSupports,
    pub tokenizer: String,
    /// Python field name is `type` (a builtin-shadowing keyword-free name).
    #[serde(rename = "type")]
    pub r#type: String,
}

/// Port of `ModelPolicy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelPolicy {
    pub state: String,
    pub terms: String,
}

/// Port of `Model`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Model {
    pub capabilities: ModelCapabilities,
    pub id: String,
    pub model_picker_enabled: bool,
    pub name: String,
    pub object: String,
    pub preview: bool,
    pub vendor: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ModelPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_endpoints: Option<Vec<String>>,
}

/// Port of `ModelsResponse`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelsResponse {
    #[serde(default)]
    pub data: Vec<Model>,
    #[serde(default)]
    pub object: String,
}

/// Passthrough helper: some providers add capability keys we don't model yet.
pub type ModelExtra = BTreeMap<String, Value>;