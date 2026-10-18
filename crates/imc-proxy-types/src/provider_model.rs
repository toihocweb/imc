//! Port of `main/proxy/lib/provider_model.py`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::types::models::Model;

/// Port of the `ProviderModelAlias` model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelAlias {
    pub model: String,
    pub provider: String,
}

/// Port of `parse_provider_model_alias` — `"provider/model"` -> alias.
pub fn parse_provider_model_alias(model: &str) -> Option<ProviderModelAlias> {
    let separator_index = model.find('/')?;
    if separator_index == 0 || separator_index == model.len() - 1 {
        return None;
    }
    let provider = model[..separator_index].trim().to_string();
    let provider_model = model[separator_index + 1..].trim().to_string();
    if provider.is_empty() || provider_model.is_empty() {
        return None;
    }
    Some(ProviderModelAlias {
        model: provider_model,
        provider,
    })
}

/// Port of `create_fallback_model`.
pub fn create_fallback_model(model_id: &str) -> Model {
    Model {
        capabilities: crate::types::models::ModelCapabilities {
            family: "provider".to_string(),
            limits: crate::types::models::ModelLimits::default(),
            object: "model_capabilities".to_string(),
            supports: crate::types::models::ModelSupports::default(),
            tokenizer: "o200k_base".to_string(),
            r#type: "chat".to_string(),
        },
        id: model_id.to_string(),
        model_picker_enabled: false,
        name: model_id.to_string(),
        object: "model".to_string(),
        preview: false,
        vendor: "provider".to_string(),
        version: "unknown".to_string(),
        policy: None,
        supported_endpoints: None,
    }
}

/// Same payload as [`create_fallback_model`] but as a raw `Value`, matching the
/// Python return type (`Dict[str, Any]`).
pub fn create_fallback_model_json(model_id: &str) -> Value {
    json!({
        "capabilities": {
            "family": "provider",
            "limits": {},
            "object": "model_capabilities",
            "supports": {},
            "tokenizer": "o200k_base",
            "type": "chat",
        },
        "id": model_id,
        "model_picker_enabled": false,
        "name": model_id,
        "object": "model",
        "preview": false,
        "vendor": "provider",
        "version": "unknown",
    })
}