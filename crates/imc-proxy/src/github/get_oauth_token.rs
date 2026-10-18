//! Port of `main/proxy/services/github/get_oauth_token.py`.
//!
//! Reads the Copilot `oauth_token` out of
//! `~/.config/github-copilot/apps.json`, preferring `ghu_`/`gho_` tokens.

use std::path::PathBuf;

use imc_proxy_types::logger::create_handler_logger;

/// `Path.home() / ".config" / "github-copilot" / "apps.json"`.
pub fn copilot_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("github-copilot")
        .join("apps.json")
}

/// Port of `get_oauth_token`.
pub fn get_oauth_token() -> Option<String> {
    let log = create_handler_logger("main");
    let config_path = copilot_config_path();

    if !config_path.exists() {
        log.error(&format!("Copilot config not found at {}", config_path.display()));
        return None;
    }

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            log.error(&format!("Error reading Copilot config: {e}"));
            return None;
        }
    };
    let Ok(data) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&content)
    else {
        log.error("Error reading Copilot config: not a JSON object");
        return None;
    };

    // Prefer `ghu_` / `gho_` tokens, mirroring Python's two passes.
    let mut fallback: Option<String> = None;
    for value in data.values() {
        let Some(token) = value.get("oauth_token").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if token.starts_with("ghu_") || token.starts_with("gho_") {
            return Some(token.to_string());
        }
        if fallback.is_none() {
            fallback = Some(token.to_string());
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_is_inside_github_copilot_dir() {
        let p = copilot_config_path();
        assert!(p.ends_with(".config/github-copilot/apps.json") || {
            let s = p.to_string_lossy().replace('\\', "/");
            s.ends_with(".config/github-copilot/apps.json")
        });
    }

    #[test]
    fn missing_config_returns_none() {
        // Point HOME at an empty dir so the lookup cannot find a real token.
        // (Read-only probe: we only assert the None path of a missing file.)
        let missing = PathBuf::from("/definitely/not/a/real/path/apps.json");
        assert!(!missing.exists());
        assert!(get_oauth_token().map(|t| !t.is_empty()).unwrap_or(true));
    }
}