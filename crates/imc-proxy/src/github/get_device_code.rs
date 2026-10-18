//! Port of `main/proxy/services/github/get_device_code.py`.

use anyhow::{Context, Result};
use serde_json::{json, Value};

use imc_proxy_types::api_config::{get_oauth_app_config, get_oauth_urls};

/// Port of `get_device_code` — start GitHub's device authorization flow.
pub async fn get_device_code() -> Result<Value> {
    let app_config = get_oauth_app_config();
    let client_id = app_config
        .get("clientId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let headers = app_config.get("headers").cloned().unwrap_or(json!({}));
    let scope = app_config
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let urls = get_oauth_urls();
    let device_code_url = urls
        .get("deviceCodeUrl")
        .cloned()
        .context("deviceCodeUrl missing")?;

    let mut header_map = reqwest::header::HeaderMap::new();
    if let Some(obj) = headers.as_object() {
        for (k, v) in obj {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v.as_str().unwrap_or("")),
            ) {
                header_map.insert(name, value);
            }
        }
    }

    let client = reqwest::Client::builder().build().context("build client")?;
    let response = client
        .post(&device_code_url)
        .headers(header_map)
        .json(&json!({ "client_id": client_id, "scope": scope }))
        .send()
        .await
        .context("post device code")?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(super::get_user::http_error(
            "Failed to get device code",
            status,
            text,
        ));
    }
    serde_json::from_str(&text).context("parse device code json")
}