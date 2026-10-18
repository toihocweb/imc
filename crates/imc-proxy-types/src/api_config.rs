//! Port of `main/proxy/lib/api_config.py` — GitHub/Copilot endpoint and header
//! constants plus the header-building helpers.
//!
//! `httpx` blocking calls in Python stay here too: the proxy already runs under
//! a blocking runtime for config-time calls, and `reqwest::blocking` is not in
//! this crate's feature set, so anything needing network I/O is left to the
//! caller crate (`imc-proxy`).

use std::collections::BTreeMap;

use crate::state::State;

pub const GITHUB_API_BASE_URL: &str = "https://api.github.com";
pub const GITHUB_BASE_URL: &str = "https://github.com";
pub const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const GITHUB_APP_SCOPES: &str = "read:user";
pub const OPENCODE_GITHUB_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";

pub const OPENCODE_VERSION: &str = "opencode/1.14.29";
pub const OPENCODE_LLM_USER_AGENT: &str = "opencode/1.14.29 ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.13, opencode/1.14.29";
pub const COPILOT_VERSION: &str = "0.58.0";
pub const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.58.0";
pub const USER_AGENT: &str = "GitHubCopilotChat/0.58.0";
pub const CLAUDE_AGENT_USER_AGENT: &str =
    "vscode_claude_code/2.1.112 (external, sdk-ts, agent-sdk/0.2.112)";
pub const COPILOT_WEBSOCKET_VERSION: &str = "0.58.0";
pub const EDITOR_WEBSOCKET_PLUGIN_VERSION: &str = "copilot-chat/0.58.0";
pub const API_VERSION: &str = "2026-06-01";
pub const WEBSOCKET_API_VERSION: &str = API_VERSION;

pub const OAUTH_APP_ENV: &str = "COPILOT_API_OAUTH_APP";
pub const ENTERPRISE_URL_ENV: &str = "COPILOT_API_ENTERPRISE_URL";

/// Port of `api_config.is_opencode_oauth_app`.
pub fn is_opencode_oauth_app() -> bool {
    std::env::var(OAUTH_APP_ENV)
        .map(|v| v.trim() == "opencode")
        .unwrap_or(false)
}

/// Port of `api_config.normalize_domain`.
pub fn normalize_domain(input_domain: &str) -> String {
    let mut domain = input_domain.trim().to_string();
    for prefix in ["https://", "http://"] {
        if let Some(rest) = domain.strip_prefix(prefix) {
            domain = rest.to_string();
        }
    }
    domain.trim_end_matches('/').to_string()
}

/// Port of `api_config.DEFAULT_ENTERPRISE_DOMAIN` (None; set via env/settings).
pub const DEFAULT_ENTERPRISE_DOMAIN: Option<&str> = None;

/// Port of `api_config.get_enterprise_domain`.
pub fn get_enterprise_domain() -> Option<String> {
    let raw = std::env::var(ENTERPRISE_URL_ENV)
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let raw = if raw.is_empty() {
        // Fallback: `cfg.enterpriseDomain` from the config store.
        crate::config_store::get_config()
            .enterprise_domain
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    } else {
        raw
    };
    if raw.is_empty() {
        return None;
    }
    let normalized = normalize_domain(&raw);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Port of `api_config.get_github_base_url`.
pub fn get_github_base_url() -> String {
    match get_enterprise_domain() {
        Some(resolved) => format!("https://{resolved}"),
        None => GITHUB_BASE_URL.to_string(),
    }
}

/// Port of `api_config.get_github_api_base_url`.
pub fn get_github_api_base_url() -> String {
    match get_enterprise_domain() {
        Some(resolved) => format!("https://api.{resolved}"),
        None => GITHUB_API_BASE_URL.to_string(),
    }
}

/// Port of `api_config.get_opencode_version` — Python has a TODO here.
pub fn get_opencode_version() -> String {
    OPENCODE_VERSION.to_string()
}

/// Port of `api_config.get_opencode_oauth_headers`.
pub fn get_opencode_oauth_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Accept".to_string(), "application/json".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("User-Agent".to_string(), get_opencode_version()),
    ])
}

/// Port of `api_config.get_opencode_llm_headers`.
pub fn get_opencode_llm_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("Accept".to_string(), "application/json".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("User-Agent".to_string(), OPENCODE_LLM_USER_AGENT.to_string()),
    ])
}

/// Port of `api_config.normalize_opencode_user_agent`.
pub fn normalize_opencode_user_agent(user_agent: &str) -> String {
    let candidate = user_agent.trim().to_string();
    let opencode_product = candidate
        .split_whitespace()
        .find(|tok| tok.starts_with("opencode/"))
        .map(|tok| tok.trim_end_matches(',').to_string());

    match opencode_product {
        Some(product) if !candidate.contains(&format!(", {product}")) => {
            format!("{candidate}, {product}")
        }
        _ => candidate,
    }
}

/// Port of `api_config.get_oauth_urls`.
pub fn get_oauth_urls() -> BTreeMap<String, String> {
    let github_base_url = get_github_base_url();
    BTreeMap::from([
        (
            "deviceCodeUrl".to_string(),
            format!("{github_base_url}/login/device/code"),
        ),
        (
            "accessTokenUrl".to_string(),
            format!("{github_base_url}/login/oauth/access_token"),
        ),
    ])
}

/// Port of `api_config.standard_headers`.
pub fn standard_headers() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("content-type".to_string(), "application/json".to_string()),
        ("accept".to_string(), "application/json".to_string()),
    ])
}

/// Port of `api_config.get_oauth_app_config`.
pub fn get_oauth_app_config() -> BTreeMap<String, serde_json::Value> {
    if is_opencode_oauth_app() {
        BTreeMap::from([
            (
                "clientId".to_string(),
                serde_json::json!(OPENCODE_GITHUB_CLIENT_ID),
            ),
            (
                "headers".to_string(),
                serde_json::json!(get_opencode_oauth_headers()),
            ),
            ("scope".to_string(), serde_json::json!(GITHUB_APP_SCOPES)),
        ])
    } else {
        BTreeMap::from([
            ("clientId".to_string(), serde_json::json!(GITHUB_CLIENT_ID)),
            ("headers".to_string(), serde_json::json!(standard_headers())),
            ("scope".to_string(), serde_json::json!(GITHUB_APP_SCOPES)),
        ])
    }
}

/// Port of `api_config.copilot_base_url`.
pub fn copilot_base_url(state: &State) -> String {
    if let Some(enterprise_domain) = get_enterprise_domain() {
        return format!("https://copilot-api.{enterprise_domain}");
    }
    if is_opencode_oauth_app() {
        return "https://api.githubcopilot.com".to_string();
    }
    if let Some(copilot_api_url) = &state.copilot_api_url {
        if !copilot_api_url.is_empty() {
            return copilot_api_url.clone();
        }
    }
    if state.account_type == "individual" {
        "https://api.githubcopilot.com".to_string()
    } else {
        format!("https://api.{}.githubcopilot.com", state.account_type)
    }
}

/// Port of `api_config.prepare_message_proxy_headers` — mutates in place.
pub fn prepare_message_proxy_headers(headers: &mut BTreeMap<String, String>) {
    if is_opencode_oauth_app() {
        return;
    }
    let request_id_value = crate::utils::random_uuid_v4();
    headers.insert("x-agent-task-id".to_string(), request_id_value.clone());
    headers.insert("x-request-id".to_string(), request_id_value.clone());
    headers.insert("x-interaction-type".to_string(), "messages-proxy".to_string());
    headers.insert("openai-intent".to_string(), "messages-proxy".to_string());
    headers.insert("user-agent".to_string(), CLAUDE_AGENT_USER_AGENT.to_string());
    remove_header_ci(headers, "copilot-integration-id");
}

/// Port of `api_config.github_user_headers`.
pub fn github_user_headers(state: &State) -> BTreeMap<String, String> {
    let token = state.github_token.clone().unwrap_or_default();
    if is_opencode_oauth_app() {
        return BTreeMap::from([
            ("Authorization".to_string(), format!("Bearer {token}")),
            ("User-Agent".to_string(), get_opencode_version()),
        ]);
    }
    BTreeMap::from([
        (
            "accept".to_string(),
            "application/vnd.github+json".to_string(),
        ),
        ("authorization".to_string(), format!("token {token}")),
        ("user-agent".to_string(), USER_AGENT.to_string()),
        ("x-github-api-version".to_string(), "2022-11-28".to_string()),
        (
            "x-vscode-user-agent-library-version".to_string(),
            "electron-fetch".to_string(),
        ),
    ])
}

/// Port of `api_config.github_headers`.
pub fn github_headers(state: &State) -> BTreeMap<String, String> {
    let token = state.github_token.clone().unwrap_or_default();
    if is_opencode_oauth_app() {
        let mut headers = BTreeMap::from([("Authorization".to_string(), format!("Bearer {token}"))]);
        headers.extend(get_opencode_oauth_headers());
        return headers;
    }
    BTreeMap::from([
        ("authorization".to_string(), format!("token {token}")),
        ("user-agent".to_string(), USER_AGENT.to_string()),
        ("x-github-api-version".to_string(), "2025-04-01".to_string()),
        (
            "x-vscode-user-agent-library-version".to_string(),
            "electron-fetch".to_string(),
        ),
    ])
}

/// Port of `api_config.github_copilot_headers`.
pub fn github_copilot_headers(
    state: &State,
    request_id: Option<&str>,
    vision: bool,
) -> BTreeMap<String, String> {
    let request_id_value = request_id
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::utils::random_uuid_v4);
    let token = state.copilot_token.clone().unwrap_or_default();
    let mut headers = BTreeMap::from([
        ("Authorization".to_string(), format!("Bearer {token}")),
        (
            "content-type".to_string(),
            standard_headers()["content-type"].clone(),
        ),
        (
            "copilot-integration-id".to_string(),
            "vscode-chat".to_string(),
        ),
        (
            "editor-device-id".to_string(),
            state.vs_code_device_id.clone(),
        ),
        (
            "editor-version".to_string(),
            format!("vscode/{}", state.vs_code_version.clone().unwrap_or_default()),
        ),
        (
            "editor-plugin-version".to_string(),
            EDITOR_PLUGIN_VERSION.to_string(),
        ),
        ("user-agent".to_string(), USER_AGENT.to_string()),
        (
            "openai-intent".to_string(),
            "conversation-agent".to_string(),
        ),
        ("x-github-api-version".to_string(), API_VERSION.to_string()),
        ("x-request-id".to_string(), request_id_value.clone()),
        (
            "x-vscode-user-agent-library-version".to_string(),
            "electron-fetch".to_string(),
        ),
        (
            "x-agent-task-id".to_string(),
            request_id_value.clone(),
        ),
        (
            "x-interaction-type".to_string(),
            "conversation-agent".to_string(),
        ),
    ]);
    if vision {
        headers.insert("copilot-vision-request".to_string(), "true".to_string());
    }
    if let Some(mac_machine_id) = &state.mac_machine_id {
        if !mac_machine_id.is_empty() {
            headers.insert("vscode-machineid".to_string(), mac_machine_id.clone());
        }
    }
    if let Some(vs_code_session_id) = &state.vs_code_session_id {
        if !vs_code_session_id.is_empty() {
            headers.insert("vscode-sessionid".to_string(), vs_code_session_id.clone());
        }
    }
    headers
}

/// Port of `api_config.copilot_models_headers`.
pub fn copilot_models_headers(state: &State) -> BTreeMap<String, String> {
    let token = state.copilot_token.clone().unwrap_or_default();
    if is_opencode_oauth_app() {
        return BTreeMap::from([
            ("Authorization".to_string(), format!("Bearer {token}")),
            ("User-Agent".to_string(), get_opencode_version()),
        ]);
    }
    let mut headers = github_copilot_headers(state, None, false);
    headers.insert("x-interaction-type".to_string(), "model-access".to_string());
    headers.insert("openai-intent".to_string(), "model-access".to_string());
    remove_header_ci(&mut headers, "x-interaction-id");
    remove_header_ci(&mut headers, "content-type");
    headers
}

/// Port of `api_config.copilot_headers`.
pub fn copilot_headers(
    state: &State,
    request_id: Option<&str>,
    vision: bool,
) -> BTreeMap<String, String> {
    let token = state.copilot_token.clone().unwrap_or_default();
    if is_opencode_oauth_app() {
        let mut headers = BTreeMap::from([
            ("Authorization".to_string(), format!("Bearer {token}")),
            ("Openai-Intent".to_string(), "conversation-edits".to_string()),
        ]);
        headers.extend(get_opencode_llm_headers());
        // request_context not ported yet in Python either, so affinity headers
        // are skipped there too.
        if vision {
            headers.insert("Copilot-Vision-Request".to_string(), "true".to_string());
        }
        return headers;
    }
    github_copilot_headers(state, request_id, vision)
}

/// Port of `api_config.get_prepared_header`.
pub fn get_prepared_header(
    headers: &BTreeMap<String, String>,
    header_name: &str,
) -> Option<String> {
    let normalized = header_name.to_lowercase();
    headers
        .iter()
        .find(|(k, _)| k.to_lowercase() == normalized)
        .map(|(_, v)| v.clone())
}

/// Port of `api_config.set_prepared_header`.
pub fn set_prepared_header(
    target: &mut BTreeMap<String, String>,
    target_name: &str,
    source: &BTreeMap<String, String>,
    source_name: &str,
) {
    if let Some(val) = get_prepared_header(source, source_name) {
        if !val.is_empty() {
            target.insert(target_name.to_string(), val);
        }
    }
}

/// Port of `api_config.omit_header`.
pub fn omit_header(headers: &BTreeMap<String, String>, header_name: &str) -> BTreeMap<String, String> {
    let normalized = header_name.to_lowercase();
    headers
        .iter()
        .filter(|(k, _)| k.to_lowercase() != normalized)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn remove_header_ci(headers: &mut BTreeMap<String, String>, header_name: &str) {
    let normalized = header_name.to_lowercase();
    headers.retain(|k, _| k.to_lowercase() != normalized);
}

/// Port of `api_config.create_header_resolver` — Python returns a two-arg
/// closure; here the returned function takes `(header_name, fallback)`.
pub fn create_header_resolver(
    headers: BTreeMap<String, String>,
) -> impl Fn(&str, &str) -> String {
    move |header_name: &str, fallback: &str| -> String {
        match get_prepared_header(&headers, header_name) {
            Some(val) => val,
            None => fallback.to_string(),
        }
    }
}

/// Port of `api_config.copilot_websocket_headers`.
pub fn copilot_websocket_headers(prepared_headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    if is_opencode_oauth_app() {
        return omit_header(prepared_headers, "x-initiator");
    }

    let request_id = get_prepared_header(prepared_headers, "x-request-id")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(crate::utils::random_uuid_v4);
    let source = create_header_resolver(prepared_headers.clone());

    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_string(), source("authorization", ""));
    headers.insert("X-Request-Id".to_string(), request_id.clone());
    headers.insert(
        "OpenAI-Intent".to_string(),
        source("openai-intent", "conversation-agent"),
    );
    headers.insert(
        "X-GitHub-Api-Version".to_string(),
        source("x-github-api-version", WEBSOCKET_API_VERSION),
    );
    headers.insert(
        "X-Interaction-Id".to_string(),
        source("x-interaction-id", &request_id),
    );
    headers.insert(
        "X-Interaction-Type".to_string(),
        source("x-interaction-type", "conversation-agent"),
    );
    headers.insert(
        "X-Agent-Task-Id".to_string(),
        source("x-agent-task-id", &request_id),
    );

    set_prepared_header(&mut headers, "VScode-SessionId", prepared_headers, "vscode-sessionid");
    set_prepared_header(&mut headers, "VScode-MachineId", prepared_headers, "vscode-machineid");

    headers.insert(
        "Editor-Device-Id".to_string(),
        source("editor-device-id", ""),
    );
    headers.insert(
        "Editor-Plugin-Version".to_string(),
        source("editor-plugin-version", EDITOR_WEBSOCKET_PLUGIN_VERSION),
    );
    headers.insert(
        "Editor-Version".to_string(),
        source("editor-version", ""),
    );
    headers.insert(
        "Copilot-Integration-Id".to_string(),
        source("copilot-integration-id", "vscode-chat"),
    );

    set_prepared_header(&mut headers, "Copilot-Vision-Request", prepared_headers, "copilot-vision-request");
    headers.insert("user-agent".to_string(), "node".to_string());
    headers
}