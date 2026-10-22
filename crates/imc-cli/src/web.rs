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
    // Globally installed layout: `imc install` puts the app here. It comes
    // before the development checkouts below, so a global install uses its own
    // copy instead of a checkout that happens to sit in $HOME.
    if let Ok(home) = std::env::var("HOME") {
        let installed = PathBuf::from(&home).join(".local/share/imc/web-dashboard");
        if installed.join("package.json").is_file() {
            dirs.push(installed);
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

/// Where the dashboard state lives (pid + the build it was started from).
fn state_path() -> PathBuf {
    imc_core::config::Config::default()
        .data_dir
        .join("dashboard_state.json")
}

/// The id of the build that is currently on disk.
fn current_build_id(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".next/BUILD_ID"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The dashboard that is running was started from an older build.
///
/// A `next start` keeps serving the build it was started with, so after a
/// `git pull` the API routes the browser talks to are the previous commit's —
/// and every call can fail while the page looks fine.
fn running_dashboard_is_stale(dir: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(state_path()) else {
        return false;
    };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let recorded = state
        .get("build_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let current = current_build_id(dir);
    !current.is_empty() && !recorded.is_empty() && recorded != current
}

/// Stop any running web dashboard. Returns true if a running dashboard was terminated.
pub fn stop_dashboard() -> bool {
    let mut terminated = false;
    // If running as a systemd service, stop the unit so it doesn't auto-restart.
    if systemctl(&["is-active", "--quiet", UNIT]) {
        if systemctl(&["stop", UNIT]) {
            terminated = true;
        }
    }
    // The pid file of a previous run belongs to the process being replaced.
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("imc-web-") && name.ends_with(".pid") {
                if let Ok(raw_pid) = std::fs::read_to_string(entry.path()) {
                    if let Ok(pid) = raw_pid.trim().parse::<i64>() {
                        let _ = Command::new("kill")
                            .arg(pid.to_string())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .status();
                        terminated = true;
                    }
                }
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    if let Ok(raw) = std::fs::read_to_string(state_path()) {
        if let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(pid) = state.get("pid").and_then(|v| v.as_i64()) {
                let _ = Command::new("kill")
                    .arg(pid.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                terminated = true;
            }
        }
        std::fs::remove_file(state_path()).ok();
    }
    // As a fail-safe, kill any orphan dashboard process listening on the default port.
    if kill_listener_on_port(DEFAULT_PORT) {
        terminated = true;
    }
    terminated
}

pub(crate) fn kill_listener_on_port(port: u16) -> bool {
    let mut killed = false;
    if let Ok(out) = Command::new("ss")
        .args(["-tulpnH", &format!("sport = :{port}")])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for part in text.split("pid=") {
            if let Some(num_str) = part.split(|c: char| !c.is_ascii_digit()).next() {
                if let Ok(pid) = num_str.parse::<i64>() {
                    if pid > 0 {
                        let _ = Command::new("kill").arg(pid.to_string()).status();
                        killed = true;
                    }
                }
            }
        }
    }
    if !killed {
        if let Ok(out) = Command::new("lsof")
            .args(["-ti", &format!("tcp:{port}"), "-sTCP:LISTEN"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Ok(pid) = line.trim().parse::<i64>() {
                    if pid > 0 {
                        let _ = Command::new("kill").arg(pid.to_string()).status();
                        killed = true;
                    }
                }
            }
        }
    }
    if killed {
        let start = std::time::Instant::now();
        while is_up(port) && start.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    killed
}

/// Stop the dashboard that `imc` started earlier.
fn stop_recorded_dashboard() {
    stop_dashboard();
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

    // 2. Start the proxy in the background. The proxy serves the embedded web dashboard
    //    on port 4789 and 12345 with zero external dependencies (no Node.js required).
    let cfg = imc_core::config::Config::default();
    if let Ok(v) = crate::claude::start_proxy(&cfg, 12345) {
        if wait_up(port, Duration::from_secs(5)) {
            let pid = v.get("pid").and_then(|p| p.as_i64()).unwrap_or(0);
            return Ok(format!("embedded dashboard via proxy (pid {pid})"));
        }
    }

    // 3. Fallback: detached `next start` if a web-dashboard source checkout exists.
    let dir = candidate_dirs()
        .into_iter()
        .next()
        .ok_or_else(|| {
            anyhow!(
                "dashboard service could not be started — run `imc` to start the background proxy and dashboard"
            )
        })?;
    let next_bin = dir.join("node_modules/next/dist/bin/next");
    let build_id = dir.join(".next/BUILD_ID");
    if !next_bin.is_file() || !build_id.is_file() {
        // A fresh clone has the app source but no dependencies and no build.
        // Building it here beats making the user remember two commands.
        build_dashboard(&dir)?;
    } else if build_is_stale(&dir, &build_id) {
        // The source changed after the last build (a `git pull`, an edit).
        // Serving the old build means the API routes the user sees are the
        // ones from the previous commit, so rebuild before starting.
        eprintln!(
            "[imc web] the dashboard source is newer than the build — rebuilding…"
        );
        build_dashboard(&dir)?;
    }
    if !next_bin.is_file() {
        return Err(anyhow!(
            "{} still has no Next.js binary after building",
            dir.display()
        ));
    }
    let node = which("node").ok_or_else(|| anyhow!("`node` not found on PATH"))?;
    let log = std::env::temp_dir().join("imc-web.log");
    let pid_file = std::env::temp_dir().join(format!("imc-web-{port}.pid"));
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)?;
    let err_file = log_file.try_clone()?;
    let child = Command::new(&node)
        .arg(&next_bin)
        .args(["start", "-p", &port.to_string(), "-H", "0.0.0.0"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(err_file))
        .spawn()?;
    let spawned_pid = child.id() as i64;
    let _ = std::fs::write(&pid_file, spawned_pid.to_string());

    if wait_up(port, Duration::from_secs(30)) {
        // Remember what is running, so a later `imc web` can tell whether it
        // is still the current build.
        let pid = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .or(Some(spawned_pid));
        let mut state = serde_json::json!({
            "pid": pid,
            "port": port,
            "build_id": current_build_id(&dir),
            "dir": dir.to_string_lossy(),
            "started_at": crate::claude::chrono_now(),
        });
        if let Some(obj) = state.as_object_mut() {
            obj.insert("pid".to_string(), serde_json::json!(pid));
        }
        let _ = std::fs::create_dir_all(
            imc_core::config::Config::default().data_dir.clone(),
        );
        let _ = std::fs::write(
            state_path(),
            serde_json::to_vec_pretty(&state).unwrap_or_default(),
        );
        Ok(format!("started `next start` from {}", dir.display()))
    } else {
        Err(anyhow!(
            "dashboard did not come up on port {port} (see {})",
            log.display()
        ))
    }
}

/// True when a source file is newer than the build.
///
/// Only the files that end up in the bundle are considered: `src/`,
/// `package.json`, the lockfile and the config files.
fn build_is_stale(dir: &Path, build_id: &Path) -> bool {
    let Ok(build_time) = std::fs::metadata(build_id)
        .and_then(|m| m.modified())
        .map_err(|e| anyhow!("stat {}: {e}", build_id.display()))
    else {
        return true;
    };
    let watched: Vec<PathBuf> = vec![
        dir.join("src"),
        dir.join("package.json"),
        dir.join("package-lock.json"),
        dir.join("next.config.ts"),
        dir.join("tsconfig.json"),
    ];
    for path in watched {
        if !path.exists() {
            continue;
        }
        let newer = if path.is_dir() {
            newest_file(&path).map(|t| t > build_time).unwrap_or(false)
        } else {
            std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|t| t > build_time)
                .unwrap_or(false)
        };
        if newer {
            return true;
        }
    }
    false
}

/// The newest modification time under a directory.
fn newest_file(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest = None;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        if let Some(t) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
            if newest.map(|n: std::time::SystemTime| t > n).unwrap_or(true) {
                newest = Some(t);
            }
        }
    }
    newest
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

/// Install dependencies and build the dashboard app.
///
/// A fresh clone ships the source but neither `node_modules` nor `.next`, so
/// `imc web` would otherwise fail with "run npm install && npm run build" —
/// exactly the manual step this command exists to remove.
fn build_dashboard(dir: &Path) -> Result<()> {
    let npm = which("npm").ok_or_else(|| {
        anyhow!("`npm` not found on PATH — install Node.js 18+ to build the dashboard")
    })?;

    if !dir.join("node_modules").is_dir() {
        // `npm ci` is reproducible when a lockfile is present.
        let has_lock = dir.join("package-lock.json").is_file();
        let args: &[&str] = if has_lock { &["ci"] } else { &["install"] };
        eprintln!(
            "[imc web] installing dashboard dependencies (npm {}) in {} — this takes a few minutes…",
            args[0],
            dir.display()
        );
        let status = Command::new(&npm)
            .args(args)
            .current_dir(dir)
            .status()
            .map_err(|e| anyhow!("failed to run npm: {e}"))?;
        if !status.success() {
            return Err(anyhow!(
                "npm {} failed in {}",
                args[0],
                dir.display()
            ));
        }
    }

    // A clean build: the old .next is removed first, so no chunk from the
    // previous build can survive in the new one.
    let next_dir = dir.join(".next");
    if next_dir.exists() {
        let _ = std::fs::remove_dir_all(&next_dir);
    }
    eprintln!("[imc web] building the dashboard (npm run build)…");
    let status = Command::new(&npm)
        .args(["run", "build"])
        .current_dir(dir)
        .status()
        .map_err(|e| anyhow!("failed to run npm: {e}"))?;
    if !status.success() {
        return Err(anyhow!("npm run build failed in {}", dir.display()));
    }
    Ok(())
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

#[allow(dead_code)]
pub(crate) fn shell_quote(s: &str) -> String {
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
    // A dashboard from an older build keeps serving the old API routes, so it
    // is replaced instead of reused.
    if is_up(port) {
        if let Some(dir) = candidate_dirs().into_iter().next() {
            if running_dashboard_is_stale(&dir) {
                eprintln!(
                    "[imc web] the running dashboard is an older build — replacing it…"
                );
                stop_recorded_dashboard();
                for _ in 0..20 {
                    if !is_up(port) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
    }
    let started = if is_up(port) || wait_up(port, Duration::from_millis(1500)) {
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
    use std::time::Duration;

    #[test]
    fn is_up_false_for_unlikely_port() {
        // 1 is reserved and nothing listens there.
        assert!(!is_up(1));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn a_fresh_build_is_not_stale() {
        let dir = std::env::temp_dir().join(format!("imc-stale-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(dir.join(".next")).unwrap();
        std::fs::write(src.join("page.tsx"), "x").unwrap();
        let build_id = dir.join(".next/BUILD_ID");
        std::fs::write(&build_id, "id").unwrap();
        // The build is written after the source.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(&build_id, "id").unwrap();
        assert!(!build_is_stale(&dir, &build_id));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_changed_source_makes_the_build_stale() {
        let dir = std::env::temp_dir().join(format!("imc-stale2-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(dir.join(".next")).unwrap();
        let build_id = dir.join(".next/BUILD_ID");
        std::fs::write(&build_id, "id").unwrap();
        // The source is written after the build.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(src.join("page.tsx"), "x").unwrap();
        assert!(build_is_stale(&dir, &build_id));
        std::fs::remove_dir_all(&dir).ok();
    }
}
