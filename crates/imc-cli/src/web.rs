//! `imc web` — open the imc dashboard in a browser, starting it first if needed.
//!
//! Resolution order for the running dashboard:
//!   1. already listening on the port → use it
//!   2. systemd unit `imc-dashboard` → `systemctl start`
//!   3. detached `next start` from the dashboard directory
//!
//! The browser is opened best-effort (`xdg-open` / `open` / `start`); on a
//! headless host the URL is printed instead so it can be opened elsewhere.

use anyhow::{anyhow, Result};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// systemd unit name for the dashboard.
const UNIT: &str = "imc-dashboard";

/// Port the dashboard listens on by default.
pub const DEFAULT_PORT: u16 = 4789;

/// Env var overriding the dashboard source directory.
const DIR_ENV: &str = "IMC_WEB_DIR";

/// Env var overriding the URL printed/opened (e.g. a public IP behind NAT).
const URL_ENV: &str = "IMC_WEB_URL";

/// Where the dashboard normally lives when the CLI and the web app are checked
/// out side by side.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var(DIR_ENV) {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        // target/release/imc -> <repo>/crates/imc-cli/... ; walk up looking for web-dashboard
        let mut cur = exe.parent().map(Path::to_path_buf);
        for _ in 0..8 {
            if let Some(p) = cur {
                let cand = p.join("web-dashboard");
                if cand.join("package.json").is_file() {
                    dirs.push(cand);
                }
                cur = p.parent().map(Path::to_path_buf);
            } else {
                break;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        for rel in ["imc-rs/web-dashboard", "imc/web-dashboard", ".imc/web-dashboard"] {
            let cand = PathBuf::from(&home).join(rel);
            if cand.join("package.json").is_file() {
                dirs.push(cand);
            }
        }
    }
    let hardcoded = PathBuf::from("/root/imc-rs/web-dashboard");
    if hardcoded.join("package.json").is_file() {
        dirs.push(hardcoded);
    }
    dirs.dedup();
    dirs
}

/// True when something accepts a TCP connection on `127.0.0.1:<port>`.
pub fn is_up(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(700)).is_ok()
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Try to bring the dashboard up. Returns a human-readable description of what
/// started it, or an error when no strategy worked.
fn start(port: u16) -> Result<String> {
    // 1. systemd unit, if the service exists.
    if systemctl(&["cat", UNIT]) {
        let _ = systemctl(&["start", UNIT]);
        if wait_up(port, Duration::from_secs(20)) {
            return Ok(format!("started systemd unit `{UNIT}`"));
        }
    }

    // 2. detached `next start`, straight from the app's own bin.
    let dir = candidate_dirs()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("dashboard directory not found (set {DIR_ENV})"))?;
    let next_bin = dir.join("node_modules/next/dist/bin/next");
    if !next_bin.is_file() {
        return Err(anyhow!(
            "{} has no built app — run `npm install && npm run build` there first",
            dir.display()
        ));
    }
    let node = which("node").ok_or_else(|| anyhow!("`node` not found on PATH"))?;
    let log = std::env::temp_dir().join("imc-web.log");
    let cmd = format!(
        "nohup {} {} start -p {} -H 0.0.0.0 >> {} 2>&1 &",
        shell_quote(&node.to_string_lossy()),
        shell_quote(&next_bin.to_string_lossy()),
        port,
        shell_quote(&log.to_string_lossy()),
    );
    let _ = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    if wait_up(port, Duration::from_secs(30)) {
        Ok(format!("started `next start` from {}", dir.display()))
    } else {
        Err(anyhow!(
            "dashboard did not come up on port {port} (see {})",
            log.display()
        ))
    }
}

fn wait_up(port: u16, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if is_up(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn which(bin: &str) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(bin);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Best-effort: hand the URL to the platform opener. Returns false when no
/// opener is available (headless host) so the caller can just print the URL.
pub fn open_in_browser(url: &str) -> bool {
    let (bin, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let mut tried = vec![bin.to_string()];
    for opener in [bin, "gio", "sensible-browser", "wslview"] {
        if which(opener).is_none() {
            continue;
        }
        tried.push(opener.to_string());
        let mut c = Command::new(opener);
        if opener == "gio" {
            c.arg("open");
        }
        c.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if c.spawn().is_ok() {
            return true;
        }
    }
    let _ = tried;
    false
}

/// Entry point for `imc web`.
pub fn run(port: u16, no_open: bool, url: Option<String>) -> Result<()> {
    let started = if is_up(port) {
        None
    } else {
        print!("dashboard not running on port {port} — starting… ");
        let _ = std::io::stdout().flush();
        let how = start(port)?;
        println!("ok");
        Some(how)
    };

    let url = url
        .or_else(|| std::env::var(URL_ENV).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("http://localhost:{port}"));

    println!("imc dashboard  →  {url}");
    if let Some(how) = started {
        println!("  ({how})");
    } else {
        println!("  (already running)");
    }

    if no_open {
        return Ok(());
    }
    if open_in_browser(&url) {
        println!("  opened in your browser");
    } else {
        println!("  no browser opener found — open the URL above manually");
        println!("  (remote host? set IMC_WEB_URL=http://<host>:{port} or use --url)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_up_false_for_unlikely_port() {
        // 1 is reserved and nothing listens there.
        assert!(!is_up(1));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }
}
