//! Port of `main/proxy/lib/config_store.py`.
//!
//! `AppConfig` uses camelCase attribute names in Python, so every struct here
//! keeps those names verbatim under `#[serde(rename_all = "camelCase")]` — the
//! JSON on disk is byte-compatible with the Python version's `model_dump()`.
//! `model_dump(exclude_none=True)` is mirrored with
//! `skip_serializing_if = "Option::is_none"`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::atomic_file::write_file_atomically;
use crate::logger::create_handler_logger;
use crate::paths::config_path;

/// Port of the `CodexReasoningEffort` Literal.
pub const CODEX_REASONING_EFFORTS: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
/// Port of the `ProviderAuthType` Literal.
pub const PROVIDER_AUTH_TYPES: &[&str] = &["authorization", "oauth2", "x-api-key"];
/// Port of the `ToolContentSupportType` Literal.
pub const TOOL_CONTENT_SUPPORT_TYPES: &[&str] = &["array", "image", "pdf"];

/// Port of `config_store.AuthConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_keys: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_api_key: Option<String>,
}

/// Port of `config_store.ContextManagementConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextManagementConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses: Option<bool>,
}

/// Port of `config_store.ResponsesTransportConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsesTransportConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers_timeout_ms_v2: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_inactivity_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_max_buffered_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_max_buffered_messages: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_open_timeout_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_pool_idle_timeout_ms: Option<i64>,
}

/// Port of `DEFAULT_RESPONSES_TRANSPORT_CONFIG`.
pub fn default_responses_transport_config() -> ResponsesTransportConfig {
    ResponsesTransportConfig {
        headers_timeout_ms_v2: Some(5 * 60 * 1000),
        stream_inactivity_timeout_ms: Some(5 * 60 * 1000),
        websocket_max_buffered_bytes: Some(8 * 1024 * 1024),
        websocket_max_buffered_messages: Some(1024),
        websocket_open_timeout_ms: Some(30_000),
        websocket_pool_idle_timeout_ms: Some(60_000),
    }
}

impl ResponsesTransportConfig {
    /// Port of the `Field(default_factory=lambda:
    /// ResponsesTransportConfig(**DEFAULT_RESPONSES_TRANSPORT_CONFIG))` default.
    pub fn with_defaults() -> Self {
        default_responses_transport_config()
    }
}

/// Port of `DEFAULT_CONTEXT_MANAGEMENT`.
pub fn default_context_management() -> ContextManagementConfig {
    ContextManagementConfig {
        messages: Some(true),
        responses: Some(false),
    }
}

/// Port of `config_store.TokenUsagePricingConfig` — empty in Python too
/// (`pass  # TODO: define based on actual usage pricing config`); kept as a
/// passthrough map so unknown pricing keys survive a load/save round-trip.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsagePricingConfig {
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Port of `config_store.ModelConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_cache: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    /// Literal["text", "image"] in Python.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_modalities: Option<Vec<String>>,
    /// Literal[...] in Python: see [`CODEX_REASONING_EFFORTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<TokenUsagePricingConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_pdf: Option<bool>,
    /// Literal[...] in Python: see [`TOOL_CONTENT_SUPPORT_TYPES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_content_support_type: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
}

/// Port of `config_store.ProviderConfig`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Literal[...] in Python: see [`PROVIDER_AUTH_TYPES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<BTreeMap<String, ModelConfig>>,
}

/// Port of `config_store.GPT5_EXPLORATION_PROMPT`.
pub const GPT5_EXPLORATION_PROMPT: &str = "## Exploration and reading files
- **Think first.** Before any tool call, decide ALL files/resources you will need.
- **Batch everything.** If you need multiple files (even from different places), read them together.
- **multi_tool_use.parallel** Use multi_tool_use.parallel to parallelize tool calls and only this.
- **Only make sequential calls if you truly cannot know the next file without seeing a result first.**
- **Workflow:** (a) plan all needed reads \u{2192} (b) issue one parallel batch \u{2192} (c) analyze results \u{2192} (d) repeat if new, unpredictable reads arise.";

/// Port of `config_store.MODEL_RESPONSES_API_COMPACT_THRESHOLDS`.
pub fn model_responses_api_compact_thresholds() -> BTreeMap<String, f64> {
    let mut m = BTreeMap::new();
    m.insert("gpt-5.4".to_string(), 272_000.0 * 0.8);
    m.insert("gpt-5.5".to_string(), 272_000.0 * 0.8);
    m
}

/// Port of `config_store.AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<BTreeMap<String, ProviderConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_mappings: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_prompts: Option<BTreeMap<String, String>>,
    #[serde(default = "default_small_model", skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    #[serde(default = "default_context_management_opt", skip_serializing_if = "Option::is_none")]
    pub context_management: Option<ContextManagementConfig>,
    #[serde(
        default = "model_responses_api_compact_thresholds_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub model_responses_api_compact_thresholds: Option<BTreeMap<String, f64>>,
    #[serde(default = "default_model_reasoning_efforts", skip_serializing_if = "Option::is_none")]
    pub model_reasoning_efforts: Option<BTreeMap<String, String>>,
    #[serde(default = "default_true_opt", skip_serializing_if = "Option::is_none")]
    pub use_messages_api: Option<bool>,
    #[serde(default = "default_true_opt", skip_serializing_if = "Option::is_none")]
    pub use_responses_api_websocket: Option<bool>,
    #[serde(
        default = "default_responses_transport_opt",
        skip_serializing_if = "Option::is_none"
    )]
    pub responses_transport: Option<ResponsesTransportConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_domain: Option<String>,
    #[serde(default = "default_true_opt", skip_serializing_if = "Option::is_none")]
    pub use_responses_api_web_search: Option<bool>,
    #[serde(default = "default_true_opt", skip_serializing_if = "Option::is_none")]
    pub alpha_search_codex_priority: Option<bool>,
    #[serde(
        default = "default_alpha_search_model",
        skip_serializing_if = "Option::is_none"
    )]
    pub alpha_search_model: Option<String>,
    #[serde(
        default = "default_message_api_web_search_model",
        skip_serializing_if = "Option::is_none"
    )]
    pub message_api_web_search_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_auto_model: Option<String>,
    #[serde(default = "default_claude_token_multiplier", skip_serializing_if = "Option::is_none")]
    pub claude_token_multiplier: Option<f64>,
}

fn default_small_model() -> Option<String> {
    Some("gpt-5-mini".to_string())
}
fn default_context_management_opt() -> Option<ContextManagementConfig> {
    Some(default_context_management())
}
fn model_responses_api_compact_thresholds_opt() -> Option<BTreeMap<String, f64>> {
    Some(model_responses_api_compact_thresholds())
}
fn default_model_reasoning_efforts() -> Option<BTreeMap<String, String>> {
    let mut m = BTreeMap::new();
    m.insert("gpt-5-mini".to_string(), "low".to_string());
    Some(m)
}
fn default_true_opt() -> Option<bool> {
    Some(true)
}
fn default_alpha_search_model() -> Option<String> {
    Some("gpt-5-mini".to_string())
}
fn default_message_api_web_search_model() -> Option<String> {
    Some("gpt-5-mini".to_string())
}
fn default_claude_token_multiplier() -> Option<f64> {
    Some(1.15)
}
fn default_responses_transport_opt() -> Option<ResponsesTransportConfig> {
    Some(default_responses_transport_config())
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auth: Some(AuthConfig::default()),
            providers: Some(BTreeMap::new()),
            model_mappings: Some(BTreeMap::new()),
            extra_prompts: Some(BTreeMap::from([(
                "gpt-5-mini".to_string(),
                GPT5_EXPLORATION_PROMPT.to_string(),
            )])),
            small_model: default_small_model(),
            context_management: default_context_management_opt(),
            model_responses_api_compact_thresholds: model_responses_api_compact_thresholds_opt(),
            model_reasoning_efforts: default_model_reasoning_efforts(),
            use_messages_api: default_true_opt(),
            use_responses_api_websocket: default_true_opt(),
            responses_transport: default_responses_transport_opt(),
            anthropic_api_key: None,
            enterprise_domain: None,
            use_responses_api_web_search: default_true_opt(),
            alpha_search_codex_priority: default_true_opt(),
            alpha_search_model: default_alpha_search_model(),
            message_api_web_search_model: default_message_api_web_search_model(),
            claude_auto_model: None,
            claude_token_multiplier: default_claude_token_multiplier(),
        }
    }
}

/// Port of `config_store.generate_admin_api_key` — `secrets.token_hex(32)`.
pub fn generate_admin_api_key() -> String {
    crate::atomic_file::random_hex(32)
}

/// Port of `config_store.ensure_config_file`.
pub fn ensure_config_file() {
    let path = config_path();
    if path.exists() {
        return;
    }
    let default_config = AppConfig::default();
    let Ok(json) = serde_json::to_string_pretty(&default_config) else {
        return;
    };
    if write_file_atomically(&path.to_string_lossy(), &json).is_ok() {
        set_owner_only_permissions(&path);
    }
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &std::path::Path) {}

/// Port of `config_store.read_config_from_disk`.
pub fn read_config_from_disk() -> AppConfig {
    ensure_config_file();
    let path = config_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return AppConfig::default();
    };
    let content = content.trim();
    if content.is_empty() {
        let default_config = AppConfig::default();
        let _ = write_config_to_disk(&default_config);
        return default_config;
    }
    match serde_json::from_str::<AppConfig>(content) {
        Ok(cfg) => cfg,
        Err(e) => {
            create_handler_logger("config_store")
                .error(&format!("Failed to read config file, using default: {e}"));
            AppConfig::default()
        }
    }
}

/// Port of `config_store.write_config_to_disk` — Python writes a `.tmp` sibling
/// and `replace()`s it; the shared atomic writer does the same thing.
pub fn write_config_to_disk(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path();
    let json = serde_json::to_string_pretty(config)?;
    let tmp = path.with_extension("tmp");
    write_file_atomically(&tmp.to_string_lossy(), &json)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn cache() -> &'static std::sync::Mutex<(Option<AppConfig>, f64)> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<(Option<AppConfig>, f64)>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new((None, 0.0)))
}

/// Port of `config_store.get_config` — mtime-gated module-level cache.
pub fn get_config() -> AppConfig {
    let path = config_path();
    let current_mtime = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let mut guard = match cache().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let (cached, last_mtime) = &mut *guard;
    if cached.is_none() || current_mtime > *last_mtime {
        *cached = Some(read_config_from_disk());
        *last_mtime = current_mtime;
    }
    cached.clone().unwrap_or_default()
}

/// Drop the cached config (used by tests and after an external write).
pub fn invalidate_config_cache() {
    let mut guard = match cache().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    *guard = (None, 0.0);
}

/// Path of the config file currently in use — handy for logging.
pub fn config_file_path() -> PathBuf {
    config_path()
}