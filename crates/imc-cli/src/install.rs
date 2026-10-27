//! `imc install` — put the current build somewhere on PATH, together with the
//! dashboard assets it needs.
//!
//! A binary copied out of the repository loses its neighbourhood: `imc web`
//! looks for `web-dashboard/` next to the executable, so a globally installed
//! imc would not find the app. This command installs both, in the layout the
//! release installer uses:
//!
//! ```text
//! ~/.local/bin/imc                     the binary
//! ~/.local/share/imc/web-dashboard/     the dashboard app
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Re-apply an ad-hoc code signature on macOS.
///
/// macOS kills a Mach-O whose signature is missing or no longer matches the
/// file — the shell just prints "killed" (SIGKILL), with no error message.
/// Two common causes: `cargo install` strips the binary, and copying a
/// signed binary invalidates the signature. `codesign --force --sign -`
/// re-signs it ad-hoc, which is enough for a local tool.
///
/// No-op on other platforms; failures are ignored (the binary may be fine).
pub fn resign_if_macos(path: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let Ok(status) = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    else {
        return false;
    };
    status.success()
}


/// Where the binary goes.
pub fn default_bin_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("IMC_INSTALL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/bin")
}

/// Where the dashboard app goes.
pub fn default_share_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/imc")
}

/// The dashboard directory inside the share dir.
pub fn installed_dashboard_dir() -> PathBuf {
    default_share_dir().join("web-dashboard")
}

/// Copy `src` → `dst`, skipping build artefacts and dependencies.
fn copy_tree(src: &Path, dst: &Path) -> Result<usize> {
    const SKIP: &[&str] = &["node_modules", ".next", ".turbo", "out", "target"];
    let mut copied = 0usize;
    std::fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if SKIP.contains(&name.as_ref()) {
            continue;
        }
        let from = entry.path();
        let to = dst.join(name.as_ref());
        if from.is_dir() {
            copied += copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copy {} → {}", from.display(), to.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Find the `web-dashboard` directory next to the running binary.
fn source_dashboard_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut cur = exe.parent().map(Path::to_path_buf);
    for _ in 0..8 {
        let p = cur?;
        let cand = p.join("web-dashboard");
        if cand.join("package.json").is_file() {
            return Some(cand);
        }
        cur = p.parent().map(Path::to_path_buf);
    }
    None
}

/// Install the current binary and (optionally) the dashboard.
///
/// Returns a report line per step so the caller can print it.
pub fn run(bin_dir: Option<PathBuf>, with_dashboard: bool) -> Result<Vec<String>> {
    let mut log = Vec::new();

    let bin_dir = bin_dir.unwrap_or_else(default_bin_dir);
    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("create {}", bin_dir.display()))?;

    let exe = std::env::current_exe().context("cannot locate the running binary")?;
    let target = bin_dir.join("imc");
    if exe != target {
        std::fs::copy(&exe, &target).with_context(|| {
            format!("copy {} → {}", exe.display(), target.display())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&target)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&target, perms)?;
        }
        if resign_if_macos(&target) {
            log.push("         re-signed (macOS ad-hoc signature)".to_string());
        }
        log.push(format!("binary  → {}", target.display()));
    } else {
        log.push(format!("binary  → {} (already there)", target.display()));
    }

    if with_dashboard {
        if let Some(src) = source_dashboard_dir() {
            let dst = installed_dashboard_dir();
            let n = copy_tree(&src, &dst)?;
            log.push(format!("dashboard → {} ({n} files)", dst.display()));

            // Reuse the dependencies and the build from the source tree: the
            // global install then runs the app that is already built instead
            // of installing node_modules a second time.
            for link in ["node_modules", ".next"] {
                let from = src.join(link);
                let to = dst.join(link);
                if from.exists() && !to.exists() {
                    #[cfg(unix)]
                    let res = std::os::unix::fs::symlink(&from, &to);
                    #[cfg(not(unix))]
                    let res: std::io::Result<()> = Err(std::io::Error::other("symlinks unsupported"));
                    if res.is_ok() {
                        log.push(format!("  {link} → reused from the source tree"));
                    }
                }
            }
        } else {
            log.push(
                "dashboard → skipped (not next to the binary; set IMC_WEB_DIR or run from a checkout)"
                    .to_string(),
            );
        }
    }

    // PATH advice
    let path = std::env::var("PATH").unwrap_or_default();
    let on_path = std::env::split_paths(&path).any(|p| p == bin_dir);
    if !on_path {
        log.push(format!(
            "note: {} is not on PATH — add it: export PATH=\"{}$PATH\"",
            bin_dir.display(),
            bin_dir.display()
        ));
    }

    // If the proxy is running, restart it with the newly installed binary
    let cfg = imc_core::config::Config::default();
    if crate::claude::proxy_is_running(&cfg) {
        let _ = crate::claude::stop_proxy(&cfg);
        let _ = crate::claude::start_proxy(&cfg, 12345);
        log.push("proxy   → restarted (port 12345)".to_string());
    }

    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_locations_live_under_home() {
        let bin = default_bin_dir();
        assert!(bin.ends_with(".local/bin"), "{}", bin.display());
        let dash = installed_dashboard_dir();
        assert!(dash.ends_with("web-dashboard"), "{}", dash.display());
    }
}
