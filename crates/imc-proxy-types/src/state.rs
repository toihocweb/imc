//! Port of `main/proxy/lib/state.py` — the process-wide singleton the Python
//! module exposes as module-level `state`. Rust callers go through
//! [`state()`] / [`with_state_mut`], which behave the same way (one shared
//! instance, mutated in place).

use std::sync::{Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};

use crate::types::models::ModelsResponse;

/// Port of the `State` pydantic model. Field names keep the Python attribute
/// spellings (camelCase) so sibling crates translate 1:1; serde mirrors that
/// wire shape exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_account_id: Option<String>,
    #[serde(default = "default_account_type")]
    pub account_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vs_code_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_machine_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vs_code_session_id: Option<String>,
    #[serde(default = "crate::utils::random_uuid_v4")]
    pub vs_code_device_id: String,
    #[serde(default)]
    pub show_token: bool,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_based_billing: Option<bool>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            github_token: None,
            user_name: None,
            copilot_token: None,
            codex_access_token: None,
            codex_refresh_token: None,
            codex_expires_at: None,
            codex_account_id: None,
            account_type: default_account_type(),
            models: None,
            vs_code_version: None,
            mac_machine_id: None,
            vs_code_session_id: None,
            vs_code_device_id: crate::utils::random_uuid_v4(),
            show_token: false,
            verbose: false,
            copilot_api_url: None,
            token_based_billing: None,
        }
    }
}

fn default_account_type() -> String {
    "individual".to_string()
}

fn global_state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

/// Port of the module-level `state` object: read the shared singleton.
/// Cloned because the lock cannot be held across await points.
pub fn state() -> State {
    global_state()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Mutable access to the shared singleton for one closure, mirroring
/// `state.githubToken = ...` assignments in Python.
pub fn with_state_mut<F: FnOnce(&mut State) -> R, R>(f: F) -> R {
    match global_state().lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poisoned) => {
            let mut guard: MutexGuard<'_, State> = poisoned.into_inner();
            f(&mut guard)
        }
    }
}