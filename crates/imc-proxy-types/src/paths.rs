//! Port of `main/proxy/lib/paths.py`.
//!
//! Python resolves these at import time from the environment; Rust resolves
//! them lazily through the same-named functions plus the [`PATHS`] struct, which
//! is what callers should use.
//!
//! Deviation: Python computes `LOCAL_CONFIG_PATH` relative to `__file__`
//! (`<repo>/hy-pro/config.json`, falling back to `<repo>/config.json`). Rust has
//! no `__file__`, so those repo-local candidates are only reachable through the
//! `COPILOT_API_CONFIG_PATH` env override; otherwise `APP_DIR/config.json` wins.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

pub const APP_DIR_ENV: &str = "COPILOT_API_HOME";
pub const AUTH_APP_ENV: &str = "COPILOT_API_OAUTH_APP";
pub const ENTERPRISE_URL_ENV: &str = "COPILOT_API_ENTERPRISE_URL";
pub const CONFIG_PATH_ENV: &str = "COPILOT_API_CONFIG_PATH";

/// Port of `paths.AUTH_APP`.
pub fn auth_app() -> String {
    std::env::var(AUTH_APP_ENV).unwrap_or_default().trim().to_string()
}

/// Port of `paths.ENTERPRISE_PREFIX` — `"ent_"` when an enterprise URL is set.
pub fn enterprise_prefix() -> &'static str {
    match std::env::var(ENTERPRISE_URL_ENV) {
        Ok(v) if !v.is_empty() => "ent_",
        _ => "",
    }
}

/// Port of `paths.DEFAULT_DIR` — `~/.local/share/copilot-api`.
pub fn default_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("copilot-api")
}

/// Port of `paths.APP_DIR`.
pub fn app_dir() -> PathBuf {
    std::env::var(APP_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_dir())
}

/// Port of `paths.GITHUB_TOKEN_PATH`.
pub fn github_token_path() -> PathBuf {
    let app = auth_app();
    app_dir()
        .join(app)
        .join(format!("{}github_token", enterprise_prefix()))
}

/// Port of `paths.CODEX_CREDENTIAL_PATH`.
pub fn codex_credential_path() -> PathBuf {
    app_dir().join("codex_credentials.json")
}

/// Port of `paths.CONFIG_PATH` resolution order.
pub fn config_path() -> PathBuf {
    match std::env::var(CONFIG_PATH_ENV) {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => app_dir().join("config.json"),
    }
}

/// Port of the `PATHS` class: same members, resolved on demand.
pub struct PATHS;

impl PATHS {
    pub fn app_dir() -> PathBuf {
        app_dir()
    }
    pub fn github_token_path() -> PathBuf {
        github_token_path()
    }
    pub fn codex_credential_path() -> PathBuf {
        codex_credential_path()
    }
    pub fn config_path() -> PathBuf {
        config_path()
    }
}

/// Port of `paths.ensure_paths`.
pub async fn ensure_paths() -> Result<()> {
    let target_dir = app_dir().join(auth_app());
    fs::create_dir_all(&target_dir)?;
    ensure_file(&github_token_path())?;
    ensure_file(&config_path())?;
    Ok(())
}

/// Port of `paths.ensure_file` — create an empty 0600 file when missing.
#[cfg(unix)]
pub fn ensure_file(file_path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if !file_path.exists() {
        fs::File::create(file_path)?;
        fs::set_permissions(file_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure_file(file_path: &Path) -> Result<()> {
    if !file_path.exists() {
        fs::File::create(file_path)?;
    }
    Ok(())
}