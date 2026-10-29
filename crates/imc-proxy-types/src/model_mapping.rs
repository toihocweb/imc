//! Port of `main/proxy/lib/model_mapping.py`.
//!
//! `check_usage` is not ported here: it calls `main/proxy/services/github/`,
//! which lives in the `imc-proxy` crate, not in this types crate. Only the
//! mapping helpers that depend on config + settings are included.

use std::collections::BTreeMap;

use crate::config_store::get_config;
use crate::logger::create_handler_logger;

/// Port of `model_mapping.get_mapped_provider` — explicit provider from a
/// mapping value, e.g. `"hyperspace:gpt-5.6-luna"` -> `"hyperspace"`.
pub fn get_mapped_provider(original_model: &str) -> Option<String> {
    let config = get_config();
    let mappings: BTreeMap<String, String> = config.model_mappings.unwrap_or_default();
    let val = mappings.get(original_model).cloned().unwrap_or_default();
    match val.split_once(':') {
        Some((prefix, _)) => Some(prefix.to_string()),
        None => None,
    }
}

/// Port of `model_mapping._get_imc_default_model` — reads the default model
/// from `~/.indexer_config.yaml` (`llm.model`).
pub fn get_imc_default_model() -> String {
    imc_core::settings::load()
        .llm
        .and_then(|l| l.model)
        .unwrap_or_default()
}

/// Prefixes Python treats as "known" model families.
pub const KNOWN_MODEL_PREFIXES: &[&str] = &[
    "claude-",
    "gpt-",
    "o1",
    "o3",
    "o4",
    "gemini",
    "gemini-",
    "google/",
    "text-embedding",
    "trajectory",
    "anthropic--",
];

/// Port of `model_mapping.apply_model_mapping`.
pub fn apply_model_mapping(model: &str) -> String {
    let config = get_config();
    let mappings: BTreeMap<String, String> = config.model_mappings.unwrap_or_default();
    let mut mapped = mappings.get(model).cloned().unwrap_or_else(|| model.to_string());
    // Strip provider prefix if present (e.g. "hyperspace:gpt-5.6-luna" -> "gpt-5.6-luna").
    if let Some((_, rest)) = mapped.split_once(':') {
        mapped = rest.to_string();
    }
    if mapped != model {
        return mapped;
    }
    // Not explicitly mapped — if the model looks unknown, fall back to the IMC
    // default. `is_hyperspace_model` lives in the hyperspace route module of the
    // `imc-proxy` crate, so callers pass it in; None means "no hyperspace
    // registry available", matching Python's empty `_hyperspace_models` set.
    if !is_hyperspace_model(model)
        && !KNOWN_MODEL_PREFIXES.iter().any(|p| model.starts_with(p))
    {
        let default = get_imc_default_model();
        if !default.is_empty() {
            create_handler_logger("main").info(&format!(
                "[Proxy] Unknown model '{model}' → IMC default '{default}'"
            ));
            return default;
        }
    }
    model.to_string()
}

/// Hook for `main/proxy/services/hyperspace/route.py::is_hyperspace_model`,
/// which the caller crate owns. Defaults to "not a hyperspace model".
pub fn is_hyperspace_model(model_id: &str) -> bool {
    match HYPERSPACE_MODEL_CHECK.get() {
        Some(check) => check(model_id),
        None => false,
    }
}

use std::sync::OnceLock;

static HYPERSPACE_MODEL_CHECK: OnceLock<Box<dyn Fn(&str) -> bool + Send + Sync>> = OnceLock::new();

/// Install the hyperspace predicate once at process start (the caller crate
/// wires this to its own registry).
pub fn set_hyperspace_model_check<F: Fn(&str) -> bool + Send + Sync + 'static>(f: F) {
    let _ = HYPERSPACE_MODEL_CHECK.set(Box::new(f));
}