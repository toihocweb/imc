//! `imc claude` — one-shot Claude Code integration: index the project, point
//! Claude Code at our own proxy (127.0.0.1:12345), inject the CLAUDE.md
//! instruction block, and run/stop the proxy. Port target for the settings +
//! instruction text: main/cli/commands/index.py::_setup_claude_code and
//! main/cli/commands/claude_templates.py.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use imc_core::config::Config;

pub const BEGIN_MARKER: &str = "<!-- imc:begin -->";
pub const END_MARKER: &str = "<!-- imc:end -->";

/// Instruction block for Claude Code: how to use imc search / find / trace.
pub fn instruction_block() -> String {
    format!(
        r#"<!-- imc:begin -->
## IndexMyCode (imc) — code intelligence for this repo
You have `imc` (a local AST + vector + call-graph index) wired in. Prefer it over blind Read/Grep for understanding code.

- **Search first**: `imc search "<question in the user's words>"` — hybrid BM25 + vector + call-graph, no LLM cost. Treat hits as source of truth; cite `file:line`.
- **Directive after a pipe**: `imc search "how indexing works | ast and memory"` — narrows facets. Chunk count: `| 8 chunks` or `--limit N`.
- **Read a file cheaply**: `imc read <file>` (compressed reader) · `imc read <file> --symbols` (outline) · `imc read <file> --method <name>` (one symbol).
- **Blast radius / callers**: `imc trace <symbol> --direction callers|callees --depth N` — before you rename or delete anything.
- **Project memory**: `imc memory-search "<topic>"` for conventions/decisions already recorded for this repo.
- If `imc search` misses 5 times, fall back to Read/Grep freely.

### Rules
1. Understand before editing: run `imc search` / `imc trace` first; don't invent APIs or requirements.
2. Cite paths (`src/foo.rs:L10-25`) when answering.
3. Keep edits surgical; match the surrounding style.
<!-- imc:end -->
"#
    )
}

/// Write `.claude/settings.local.json` so Claude Code talks to our proxy.
/// Port of `_setup_claude_code` (index.py) — env values kept identical so a
/// user's existing Claude setup keeps working.
pub fn write_claude_settings(project_root: &Path, port: u16) -> Result<PathBuf> {
    let claude_dir = project_root.join(".claude");
    std::fs::create_dir_all(&claude_dir)
        .with_context(|| format!("create {}", claude_dir.display()))?;
    let path = claude_dir.join("settings.local.json");

    let mut data: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(|v: &Value| v.is_object())
        .unwrap_or_else(|| json!({}));

    let obj = data.as_object_mut().expect("object");
    // Preserve any env the user already set; only our keys are overwritten.
    let env = obj
        .entry("env")
        .or_insert_with(|| json!({}));
    if !env.is_object() {
        *env = json!({});
    }
    let env = env.as_object_mut().expect("env object");
    for (k, v) in [
        ("ANTHROPIC_AUTH_TOKEN", json!("dummy")),
        ("ANTHROPIC_BASE_URL", json!(format!("http://127.0.0.1:{port}"))),
        ("ANTHROPIC_MODEL", json!("claude-pro-max")),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", json!("claude-pro-max")),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", json!("claude-pro-max")),
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", json!("claude-pro-max")),
    ] {
        env.insert(k.to_string(), v);
    }

    let mut text = serde_json::to_string_pretty(&data)?;
    text.push('\n');
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Insert or replace the imc block in the project's CLAUDE.md.
pub fn write_claude_md(project_root: &Path) -> Result<(PathBuf, bool)> {
    let path = project_root.join("CLAUDE.md");
    let block = instruction_block();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let updated = if let (Some(b), Some(e)) = (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) {
        if b < e {
            let mut s = existing.clone();
            s.replace_range(b..e + END_MARKER.len(), &block);
            s
        } else {
            format!("{existing}\n{block}")
        }
    } else {
        if existing.trim().is_empty() {
            block.clone()
        } else {
            format!("{}\n{block}", existing.trim_end())
        }
    };
    let changed = updated != existing;
    std::fs::write(&path, updated).with_context(|| format!("write {}", path.display()))?;
    Ok((path, changed))
}

fn state_path(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("proxy_state.json")
}

/// Path to the proxy daemon binary (sits next to the `imc` binary).
pub fn proxy_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locate imc binary")?;
    let cand = exe.with_file_name("imc-proxyd");
    if cand.exists() {
        return Ok(cand);
    }
    // `cargo run` puts it in target/{debug,release}/ alongside `imc`; fall back to PATH.
    anyhow::bail!(
        "imc-proxyd not found next to {} — build it first: cargo build --release --bin imc-proxyd",
        exe.display()
    )
}

/// Spawn the proxy in the background and record pid/port.
pub fn start_proxy(cfg: &Config, port: u16) -> Result<Value> {
    let exe = proxy_bin()?;
    let exe_disp = exe.display().to_string();
    let log_dir = cfg.data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log = std::fs::File::create(log_dir.join("proxy.log"))
        .with_context(|| format!("create {}", log_dir.join("proxy.log").display()))?;
    let err = log.try_clone()?;
    let child = std::process::Command::new(&exe)
        .args(["--port", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(err))
        .spawn()
        .with_context(|| format!("spawn `{exe_disp} --port {port}`"))?;
    let pid = child.id();
    let state = json!({
        "pid": pid,
        "port": port,
        "started_at": chrono_now(),
        "upstream": {
            "base_url": cfg.llm_base_url(),
            "model": cfg.llm_model(),
            "api_key_set": !cfg.llm_api_key().is_empty(),
        }
    });
    std::fs::create_dir_all(&cfg.data_dir).ok();
    std::fs::write(state_path(cfg), serde_json::to_vec_pretty(&state)?)?;
    Ok(state)
}

pub fn stop_proxy(cfg: &Config) -> Result<Option<u16>> {
    let p = state_path(cfg);
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return Ok(None);
    };
    let state: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    let pid = state.get("pid").and_then(|v| v.as_i64());
    let port = state.get("port").and_then(|v| v.as_u64()).map(|v| v as u16);
    if let Some(pid) = pid {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    std::fs::remove_file(&p).ok();
    Ok(port)
}

pub fn proxy_status(cfg: &Config) -> Option<Value> {
    let raw = std::fs::read_to_string(state_path(cfg)).ok()?;
    let state: Value = serde_json::from_str(&raw).ok()?;
    let port = state.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    Some(json!({ "running": port_open(port), "state": state }))
}

fn port_open(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Full setup: index → Claude settings → CLAUDE.md → start proxy.
pub fn setup_and_start(
    cfg: &Config,
    root: &Path,
    port: u16,
    force_index: bool,
) -> Result<Vec<String>> {
    let mut log = Vec::new();

    if !root.is_dir() {
        bail!("not a directory: {}", root.display());
    }

    // 1. Index (incremental unless forced).
    let stats = imc_index::run(cfg, root, &[], force_index)
        .context("index project")?;
    log.push(format!(
        "indexed {} files (new {}, modified {}, unchanged {}) in {} ms — {} symbols, {} edges, {} chunks",
        stats.total_files,
        stats.new_files,
        stats.modified_files,
        stats.unchanged_files,
        stats.elapsed_ms,
        stats.symbols,
        stats.edges,
        stats.chunks
    ));

    // 2. Register project + make it active.
    let meta = imc_core::config::ProjectMeta {
        id: imc_core::hash::md5_hex(root.to_string_lossy().as_bytes())[..16].to_string(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".into()),
        path: root.to_string_lossy().to_string(),
        indexed_at: Some(chrono_now()),
        file_count: Some(stats.total_files as u64),
    };
    imc_core::config::upsert_project(cfg, meta)?;
    cfg.set_active(root)?;

    // 3. Claude Code settings + CLAUDE.md.
    let settings = write_claude_settings(root, port)?;
    log.push(format!("wrote {}", settings.display()));
    let (md, changed) = write_claude_md(root)?;
    log.push(format!(
        "{} {}",
        if changed { "updated" } else { "wrote" },
        md.display()
    ));

    // 4. Proxy.
    let key = cfg.llm_api_key();
    if key.is_empty() {
        log.push(format!(
            "WARNING: no upstream key — run `imc config set llm.api_key <key>` (see {})",
            imc_core::settings::path().display()
        ));
    }
    let state = start_proxy(cfg, port)?;
    log.push(format!(
        "proxy running: pid={} http://127.0.0.1:{} → upstream {}",
        state["pid"], state["port"], state["upstream"]["base_url"]
    ));
    log.push("done — just run `claude` in this directory".to_string());
    Ok(log)
}

/// Unix seconds; keeps state files human-comparable.
pub fn chrono_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn print_stderr(msg: &str) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "{msg}");
}