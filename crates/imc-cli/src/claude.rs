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
    // Kept in sync with docs/search_prompt.md. Every command named here is
    // verified to exist in this binary (e2e/prompt_claims_check).
    format!(
        r#"<!-- imc:begin -->
## imc — code intelligence for this repo

Local AST + call-graph + vector index. Prefer it over blind Read/Grep to understand
code. Every command below is verified to exist in this binary.

**Find code**
- `imc search "<3-7 words>"` — answers the question from indexed chunks with an
  LLM (needs `llm.base_url`), citing `path:line-line`. Without an LLM endpoint it
  prints the ranked hits instead and says so.
- `imc search "<q>" --raw` — just the ranked `file:start-end` chunks, no LLM.
  Measured on this repo: top-1 hit is right 52% of the time, top-5 is right 88%.
  **Always read the top 5, never stop at hit 1.**
- `imc search "<exact identifier>"` — for a known symbol this is exact, not fuzzy:
  case and `_`/camelCase are normalised on both sides. Prefer it when you know the name.
- `imc search "<q>" --json` — machine-readable hits (implies `--raw`).
- `imc search "<q>" --no-vector` — BM25 only; use when no embedding endpoint is
  configured or when you want deterministic, offline results.
- Add `-C <project-root>` when the current directory may not be the project:
  without it imc answers for the *globally active* project, so a stale pointer
  silently returns another repository's files. `-p <id_or_name>` picks a
  registered project instead of a path.

**Read less**
- `imc read <file> --symbols` — outline first, then read only the lines you need.
- `imc read <file> --method <name>` — one symbol's body. **Use the real name from
  `--symbols`, including any leading underscore** (`_blended_score`, not `blended_score`).
- `imc context <file>` — the whole feature around a file: component, deps, tests.

**Understand impact**
- `imc trace <symbol> --callers --depth 3` — blast radius. Run this *before* renaming
  or deleting anything. `imc callers <symbol>` and `imc blast <symbol>` are shorthands.
- `imc trace <symbol>` — what it calls (forward). `--both` walks both directions.
- `imc trace "GET /api/path"` — where an HTTP route goes: frontend caller to backend
  handler.
- `imc graph <symbol>` — definition site and kind.

**Recall past decisions**
- `imc memory-search "<topic>"` — conventions and decisions previously recorded
  for this repo. Empty is a valid answer.

**Rules**
1. Search before you edit; never invent an API, path, or line number.
2. Cite `path/to/file:L10-L25` when answering. Never cite a range you did not see.
3. `imc trace` has no relevance floor: a junk query still returns hits. Hits are
   ranked evidence, not proof — open the file before relying on it.
4. If search misses repeatedly, fall back to Read/Grep. An empty result means
   "not in the index", not "does not exist".
<!-- imc:end -->"#
    )
}

/// Rule file for Claude Code: project-local rules that tell Claude how to use
/// imc. Written to `.claude/rules/imc-rule.md` by `imc claude setup` (and
/// `imc config setup` on request) instead of being stuffed into CLAUDE.md.
pub fn imc_rule_md() -> &'static str {
    r#"# imc — code-intelligence rules for this repo

You have access to the `imc` CLI (Rust port of IndexMyCode) in this repo.
Prefer `imc` over blind Read/Grep when you need to understand code, find
callers, or trace HTTP routes.

## Commands you should use

- `imc search "<3-7 words>"` — LLM answer from indexed chunks, with `path:line`
  citations. Needs `llm.base_url` configured. Without an LLM endpoint it prints
  ranked hits instead.
- `imc search "<q>" --raw` — raw ranked chunks, no LLM. Always read top 5,
  never stop at hit 1.
- `imc search "<exact symbol>"` — exact symbol lookup (case & `_`/camelCase
  normalized). Use when you know the name.
- `imc read <file> --symbols` — outline; then read only the lines you need.
- `imc context <file>` — whole feature around a file: deps, endpoints, tests.
- `imc trace <symbol> --callers --depth 3` — blast radius; run *before* you
  rename or delete anything. `imc callers <symbol>` / `imc blast <symbol>`.
- `imc trace "<symbol>"` — what it calls (forward). `--both` both directions.
- `imc trace "GET /api/path"` — HTTP route → backend handler.
- `imc graph <symbol>` — definition site and kind.
- `imc memory-search "<topic>"` — conventions/decisions recorded for this repo.
  There is a relevance floor: an empty result means "no matching memory", not
  "this repo has no memories".

If MCP tools are available (`mcp__imc__search`, `mcp__imc__trace`, `mcp__imc__read`,
`mcp__imc__context`, `mcp__imc__memory_search`, `mcp__imc__pack`), prefer them over
shelling out: they carry the schema, the arguments are validated, and the project
root is pinned. Fall back to the CLI (`imc ...`) when MCP is not wired.

## Rules

1. **Search before you edit.** Never invent an API, path, or line number.
2. **Cite `path/to/file:L10-L25`** when answering. Never cite a range you did
   not see in an imc output.
3. **`imc trace` has no relevance floor.** A junk query still returns hits.
   Hits are ranked evidence, not proof — open the file before relying on it.
4. **If search misses repeatedly, fall back to Read/Grep.** An empty result
   means "not in the index", not "does not exist".
0. **A bare `imc` starts the proxy and the dashboard** (the one-command
   setup). `imc claude` does the same and then runs the `claude` binary.
   `imc .` indexes the current directory.
5. **Use `-C <project-root>`** (or `-p <id>`) when the cwd may not be the
   project; otherwise imc answers for the globally active project and says so
   on stderr ("using the active project …"). `imc index` without a path always
   indexes the current directory, never the active project.

## Setup

- `imc config setup` — interactive setup of LLM URL, API key, models.
- `imc config test` — verify the embedding/chat endpoint.
- `imc claude setup` — index, wire `.claude/settings.local.json`, add this
  rule file, and start the proxy.
- `imc web` — open the web dashboard (starts it if needed); `--url` or
  `IMC_WEB_URL` for a remote host.

"#
}

/// Write `.claude/rules/imc-rule.md` — the project-local imc rules.
pub fn write_imc_rule(project_root: &Path) -> Result<PathBuf> {
    let rules_dir = project_root.join(".claude").join("rules");
    std::fs::create_dir_all(&rules_dir)
        .with_context(|| format!("create {}", rules_dir.display()))?;
    let path = rules_dir.join("imc-rule.md");
    std::fs::write(&path, imc_rule_md().to_string()).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
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

/// The proxy runs inside the `imc` binary (`imc proxy serve`), so there is no
/// second executable to look for. The binary itself is what gets spawned in
/// the background.
pub fn proxy_bin() -> Result<PathBuf> {
    std::env::current_exe().context("locate imc binary")
}

/// The daemon's file modification time, in seconds since the epoch.
fn file_mtime_secs(path: &std::path::Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// Spawn the proxy in the background and record pid/port.
///
/// The proxy keeps serving the code it was started with, and the
/// request-path fixes live in the proxy — not in the CLI. So a server older
/// than the binary on disk is replaced instead of reused.
pub fn start_proxy(cfg: &Config, port: u16) -> Result<Value> {
    let exe = proxy_bin()?;
    if daemon_is_stale(cfg, &exe) {
        let _ = stop_proxy(cfg);
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    let exe_disp = exe.display().to_string();
    let log_dir = cfg.data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log = std::fs::File::create(log_dir.join("proxy.log"))
        .with_context(|| format!("create {}", log_dir.join("proxy.log").display()))?;
    let err = log.try_clone()?;
    let child = std::process::Command::new(&exe)
        .args(["proxy", "serve", "--port", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(err))
        .spawn()
        .with_context(|| format!("spawn `{exe_disp} proxy serve --port {port}`"))?;
    let pid = child.id();
    let state = json!({
        "pid": pid,
        "port": port,
        "started_at": chrono_now(),
        "server_path": exe_disp,
        "server_mtime": file_mtime_secs(&exe),
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

/// True when the running proxy was started from an older build.
fn daemon_is_stale(cfg: &Config, exe: &std::path::Path) -> bool {
    let Some(status) = proxy_status(cfg) else {
        return false;
    };
    if !status
        .get("running")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }
    let recorded = status
        .get("state")
        .and_then(|s| s.get("server_mtime"))
        .and_then(|v| v.as_u64());
    let current = file_mtime_secs(exe);
    match (recorded, current) {
        // A daemon started before this field existed: treat it as stale when
        // the binary on disk is newer than the recorded start time.
        (None, Some(cur)) => {
            let started = status
                .get("state")
                .and_then(|s| s.get("started_at"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            cur > started
        }
        (Some(rec), Some(cur)) => cur > rec,
        _ => false,
    }
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
    let rule = write_imc_rule(root)?;
    log.push(format!("wrote {}", rule.display()));

    // MCP discovery: .mcp.json lets any MCP client see the imc tools. The
    // running proxy serves the same server over HTTP at
    // http://127.0.0.1:12345/mcp.
    let mcp = crate::mcp::install(root, None)?;
    log.push(format!(
        "wrote {} (MCP tools: search, ask, trace, read, context, memory_search, pack)",
        mcp.display()
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

/// Run the `claude` binary directly, passing all args through.
/// Ensures the imc proxy is running first (starts it on port 12345 if needed),
/// then spawns `claude` detached so it survives terminal close.
pub fn run_claude_bin(cfg: &Config, _root: &Path, args: &[String]) -> Result<()> {
    // Ensure the proxy is running so Claude Code talks to our compression
    // proxy instead of hitting the upstream LLM directly.
    if !proxy_is_running(cfg) {
        let port = 12345u16;
        let state = start_proxy(cfg, port)
            .with_context(|| format!("start imc proxy on port {port}"))?;
        println!(
            "proxy auto-started: pid={} http://127.0.0.1:{}",
            state["pid"], state["port"]
        );
    } else {
        println!("proxy already running on http://127.0.0.1:12345");
    }

    let exe = which_claude_bin()?;
    let exe_disp = exe.display().to_string();

    let log_dir = imc_core::settings::path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("claude_sessions");
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("create {}", log_dir.display()))?;
    let log = std::fs::File::create(log_dir.join("claude.log"))
        .with_context(|| format!("create {}", log_dir.join("claude.log").display()))?;
    let err = log.try_clone()?;

    // Build the command with detached flags so it survives terminal close.
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(err));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // new process group, survives terminal close
    }

    let child = cmd.spawn().with_context(|| format!("spawn `{exe_disp}`"))?;
    let pid = child.id();

    // Do not wait — return immediately. The child keeps running.
    std::mem::forget(child);

    println!("claude started: pid={} (background, survives terminal close)", pid);
    Ok(())
}

/// Check whether the imc proxy is currently reachable on port 12345.
pub fn proxy_is_running(cfg: &Config) -> bool {
    proxy_status(cfg).map_or(false, |s| {
        s.get("running").and_then(|v| v.as_bool()).unwrap_or(false)
    })
}

/// Locate the `claude` binary: check `IMC_CLAUDE_BIN` env, then PATH.
fn which_claude_bin() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("IMC_CLAUDE_BIN") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    // Search PATH.
    if let Some(p) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&p) {
            let cand = dir.join("claude");
            if cand.exists() && cand.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = cand.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            return Ok(cand);
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    return Ok(cand);
                }
            }
        }
    }
    bail!("claude binary not found — set IMC_CLAUDE_BIN or install Claude Code and add it to PATH")
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