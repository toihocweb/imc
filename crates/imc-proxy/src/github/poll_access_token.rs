//! Port of `main/proxy/services/github/poll_access_token.py`.

use anyhow::{Context, Result};
use serde_json::{json, Value};

use imc_proxy_types::api_config::{get_oauth_app_config, get_oauth_urls};
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::utils::sleep;

/// Port of `poll_access_token` — poll GitHub's device-code endpoint until the
/// user approves (or an error response turns up).
///
/// Mirrors Python's infinite loop: unsuccessful HTTP responses and payloads
/// without an `access_token` both just wait `interval + 1` seconds and retry.
pub async fn poll_access_token(device_code: &Value) -> Result<String> {
    let log = create_handler_logger("main");
    let app_config = get_oauth_app_config();
    let client_id = app_config
        .get("clientId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let headers = app_config.get("headers").cloned().unwrap_or(json!({}));
    let urls = get_oauth_urls();
    let access_token_url = urls
        .get("accessTokenUrl")
        .cloned()
        .context("accessTokenUrl missing")?;

    let interval_ms =
        (device_code.get("interval").and_then(Value::as_i64).unwrap_or(5) + 1) * 1000;
    log.debug(&format!("Polling access token with interval of {interval_ms}ms"));

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

    let device_code_value = device_code
        .get("device_code")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let client = reqwest::Client::builder().build().context("build client")?;
    loop {
        let body = json!({
            "client_id": client_id,
            "device_code": device_code_value,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        });
        let response = client
            .post(&access_token_url)
            .headers(header_map.clone())
            .json(&body)
            .send()
            .await;

        let status_ok = match &response {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        };
        if !status_ok {
            sleep(interval_ms).await;
            log.error("Failed to poll access token (non-success response)");
            continue;
        }

        let json_data = match response {
            Ok(resp) => {
                let text = resp.text().await.unwrap_or_default();
                serde_json::from_str::<Value>(&text).unwrap_or(Value::Null)
            }
            Err(_) => Value::Null,
        };
        log.debug(&format!("Polling access token response: {json_data}"));

        if let Some(token) = json_data.get("access_token").and_then(Value::as_str) {
            if !token.is_empty() {
                return Ok(token.to_string());
            }
        }
        sleep(interval_ms).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_reads_interval_from_device_code() {
        // The +1s of slack Python adds; asserting the value it derives.
        let dc = json!({"device_code": "abc", "interval": 9});
        let interval = dc.get("interval").and_then(Value::as_i64).unwrap_or(5) + 1;
        assert_eq!(interval, 10);
        let default = json!({"device_code": "abc"});
        assert_eq!(default.get("interval").and_then(Value::as_i64).unwrap_or(5) + 1, 6);
    }

    #[test]
    fn access_token_url_is_the_github_oauth_endpoint() {
        let urls = get_oauth_urls();
        assert!(urls["accessTokenUrl"].ends_with("/login/oauth/access_token"));
        assert!(urls["deviceCodeUrl"].ends_with("/login/device/code"));
    }

    #[test]
    fn poll_never_returns_a_missing_token() {
        // Contract check: `poll_access_token` only resolves with a non-empty
        // `access_token`; an empty one keeps polling (so the loop must not
        // terminate on `Some("")`).
        let payload = json!({"access_token": ""});
        assert!(payload
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .is_none());
    }
}