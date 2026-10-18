//! Port of `main/proxy/services/github/get_user.py`.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::Value;

use imc_proxy_types::api_config::{get_github_api_base_url, github_user_headers};
use imc_proxy_types::error::HTTPError;
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::state::{state, State};

/// Build the `model_copy(update={"githubToken": ...})` state Python passes to
/// the header builders.
pub(crate) fn auth_state(github_token: Option<&str>) -> State {
    let mut st = state();
    if let Some(token) = github_token.filter(|t| !t.is_empty()) {
        st.github_token = Some(token.to_string());
    }
    st
}

/// Python-style `Dict[str, str]` -> `reqwest` header map.
pub(crate) fn header_map(headers: &BTreeMap<String, String>) -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            map.insert(name, value);
        }
    }
    map
}

pub(crate) fn http_error(
    message: &'static str,
    status: u16,
    text: String,
) -> anyhow::Error {
    Box::new(HTTPError::new(message, status, text, None)).into()
}

/// Port of `get_github_user`.
pub async fn get_github_user(github_token: Option<&str>) -> Result<Value> {
    let log = create_handler_logger("main");
    let st = state();
    let resolved = github_token
        .or_else(|| st.github_token.as_deref().filter(|t| !t.is_empty()))
        .ok_or_else(|| anyhow::anyhow!("GitHub token not found"))?
        .to_string();

    let url = format!("{}/user", get_github_api_base_url());
    let headers = github_user_headers(&auth_state(Some(&resolved)));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build http client")?;
    let response = client
        .get(&url)
        .headers(header_map(&headers))
        .send()
        .await
        .context("get github user")?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        log.error(&format!("Failed to get Github user response body: {text}"));
        return Err(http_error("Failed to get GitHub user", status, text));
    }

    serde_json::from_str(&text).context("parse github user json")
}