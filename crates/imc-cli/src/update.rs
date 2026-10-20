//! Version check and self-update.
//!
//! `imc update` downloads the newest release asset and replaces the running
//! binary. Independently, every command runs a cheap cached check: if the
//! cache says a newer release exists, imc prints a one-line notice, and a
//! stale cache is refreshed by a detached background process so the next run
//! knows without slowing this one down.
//!
//! Escape hatches: `IMC_NO_UPDATE_CHECK=1` disables it entirely,
//! `IMC_UPDATE_REPO=<owner/name>` points at a fork.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use imc_core::config::Config;

/// GitHub repository that publishes the releases.
pub fn repo() -> String {
    std::env::var("IMC_UPDATE_REPO").unwrap_or_else(|_| "toihocweb/imc".to_string())
}

fn api_latest() -> String {
    format!("https://api.github.com/repos/{}/releases/latest", repo())
}

/// Version of this build.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// How long a fetched result stays fresh.
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// How long a cached "there is a newer version" notice stays believable.
const NOTICE_MAX_AGE_SECS: u64 = 7 * 24 * 60 * 60;

/// Where the check result is cached.
pub fn cache_path() -> PathBuf {
    Config::default().data_dir.join("update_check.json")
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CheckCache {
    /// Unix seconds of the last successful check.
    pub checked_at: u64,
    /// Latest version seen on the server (without a leading `v`).
    pub latest: String,
}

/// Read the cached result.
pub fn read_cache() -> Option<CheckCache> {
    let raw = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the cache, ignoring failures (a read-only home must not break imc).
pub fn write_cache(cache: &CheckCache) {
    if let Some(parent) = cache_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(cache) {
        let _ = std::fs::write(cache_path(), body);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Compare dotted versions numerically: `0.2.0 > 0.1.9`. A leading `v` and
/// any pre-release suffix are ignored.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let mut l = parse(latest);
    let mut c = parse(current);
    let n = l.len().max(c.len());
    l.resize(n, 0);
    c.resize(n, 0);
    l > c
}

/// Ask GitHub for the latest published version.
pub fn fetch_latest(timeout: Duration) -> Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent("imc-update-check")
        .build()
        .into();
    let mut res = agent
        .get(&api_latest())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| anyhow::anyhow!("could not reach GitHub: {e}"))?;
    let body_text = res.body_mut().read_to_string().unwrap_or_default();
    let body: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
    let tag = body
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("release has no tag_name"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Refresh the cache. Returns the latest version seen.
pub fn refresh(timeout: Duration) -> Result<String> {
    let latest = fetch_latest(timeout)?;
    write_cache(&CheckCache {
        checked_at: now(),
        latest: latest.clone(),
    });
    Ok(latest)
}

/// `linux-x86_64`, `darwin-aarch64`, … — matches the release asset names.
pub fn platform() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        ("macos", "aarch64") => Some("darwin-aarch64"),
        _ => None,
    }
}

/// Download URL of the release asset for this platform.
pub fn asset_url(tag: &str) -> Option<String> {
    let plat = platform()?;
    Some(format!(
        "https://github.com/{}/releases/download/v{}/imc-{plat}",
        repo(),
        tag
    ))
}

/// Should the automatic check run at all?
pub fn enabled() -> bool {
    if std::env::var("IMC_NO_UPDATE_CHECK").is_ok() {
        return false;
    }
    if std::env::var("CI").is_ok() {
        return false;
    }
    // The refresh child must not spawn another refresh.
    !std::env::var("IMC_UPDATE_CHECK_CHILD").is_ok()
}

/// The notice to show for this run, if any.
///
/// Reads the cache only — no network on the hot path.
pub fn notice() -> Option<String> {
    let cache = read_cache()?;
    if now().saturating_sub(cache.checked_at) > NOTICE_MAX_AGE_SECS {
        return None;
    }
    if is_newer(&cache.latest, current_version()) {
        Some(format!(
            "update available: v{} → v{} · run `imc update`",
            current_version(),
            cache.latest
        ))
    } else {
        None
    }
}

/// Spawn a detached refresh so the *next* run knows about new releases.
pub fn spawn_background_refresh() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("update")
        .arg("--check")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.env("IMC_UPDATE_CHECK_CHILD", "1");
    // Detach so the child survives this process.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

/// Run the automatic check: print a notice when the cache knows of a newer
/// release, and refresh a stale cache in the background.
pub fn maybe_notify() {
    if !enabled() {
        return;
    }
    let stale = match read_cache() {
        Some(c) => now().saturating_sub(c.checked_at) > CHECK_INTERVAL_SECS,
        None => true,
    };
    if stale {
        spawn_background_refresh();
    }
    if let Some(msg) = notice() {
        eprintln!("[imc] {msg}");
    }
}

/// Download the latest release and replace the running binary.
pub fn install(timeout: Duration) -> Result<(String, PathBuf)> {
    let latest = fetch_latest(timeout)?;
    if !is_newer(&latest, current_version()) {
        bail!("already on the latest version (v{})", current_version());
    }
    let url = asset_url(&latest)
        .ok_or_else(|| anyhow::anyhow!("no release asset for this platform ({})", platform().unwrap_or("?")))?;

    let exe = std::env::current_exe().context("cannot locate the running binary")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent("imc-update")
        .build()
        .into();
    let mut res = agent
        .get(&url)
        .call()
        .map_err(|e| anyhow::anyhow!("download failed: {e}"))?;
    if res.status() != 200 {
        bail!("download failed: HTTP {}", res.status());
    }
    let bytes = res.body_mut().read_to_vec()?;

    let staged = exe.with_extension("new");
    std::fs::write(&staged, &bytes).with_context(|| format!("write {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staged)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged, perms)?;
    }
    std::fs::rename(&staged, &exe)
        .with_context(|| format!("replace {}", exe.display()))?;
    Ok((latest, exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.2", "0.1.1"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.1", "0.1.0"));
    }

    #[test]
    fn strips_release_suffixes() {
        assert!(is_newer("0.2.0-rc1", "0.1.9"));
    }

    #[test]
    fn platform_is_known_on_linux_and_macos() {
        let p = platform();
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(p.is_some(), "expected a release asset platform");
        }
    }
}
