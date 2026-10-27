//! Claude Code integration: point Claude Code at our own proxy
//! (127.0.0.1:12345), write the rule file, and hand the terminal over to
//! the `claude` binary.
//!
//! `imc claude` is a passthrough — every flag and every subcommand belongs
//! to Claude Code, because Claude Code has subcommands of its own (`stop`,
//! `mcp`, `doctor`, ...) and a second set of them here would collide.
//! `imc setup` does the same wiring without opening Claude Code.
//!
//! Port target for the settings + instruction text:
//! main/cli/commands/index.py::_setup_claude_code and
//! main/cli/commands/claude_templates.py.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use imc_core::config::Config;

pub const BEGIN_MARKER: &str = "<!-- imc:begin -->";
pub const END_MARKER: &str = "<!-- imc:end -->";

/// Rule file for Claude Code: project-local rules that tell Claude how to use
/// imc. Written to `.claude/rules/imc-rule.md` by `imc setup` (and
/// `imc config setup` on request) instead of being stuffed into CLAUDE.md.
pub fn imc_rule_md() -> &'static str {
    r#"# imc — code intelligence for this repo

This repo is indexed by `imc`: an AST symbol index, a call graph, a lexical/vector
index, and a durable memory store. Prefer `imc` over blind Read/Grep.

## Critical: Understand & Clarify Before Coding (Clarification Gate)

NEVER start writing code, editing files, or running destructive commands on an ambiguous or underspecified request. Follow this strict protocol before any implementation:
1. **Investigate first**: Run `imc search "<query>" --raw` or `imc read <file> --symbols` to inspect existing code and architecture. Never ask questions that the codebase already answers.
2. **Confirm understanding**: Present a concise 1-2 sentence summary of your technical understanding of the problem and desired outcome.
3. **Ask clarifying questions**: If requirements, edge cases, error handling, or scope boundaries have any ambiguity, ask 1-3 direct, concrete questions (or provide options) and WAIT for confirmation before touching files.
4. **Zero blind edits**: Do NOT call `Edit` or `Write` on the first turn of a new task without explicit confirmation or clear, unambiguous requirements.

## Standard Workflow

1. **Orient**: Start with `imc pack "<task or feature>"` to get an all-in-one bundle of code chunks, call-graph connections, and repo conventions under a token budget.
2. **Find**: Use `imc search "<query>" --raw` for direct ranked code chunks (bypasses LLM summarization). Use `--concise` for a quick 1-line overview.
3. **Inspect**: Never read whole files. Run `imc read <file> --skeleton` to view API types and signatures, or `imc read <file> --symbols` for an outline, then `imc read <file> --method <name>` to read only the target function/method body.
4. **Impact check**: Before editing or deleting any function, run `imc trace <symbol> --callers --depth 3` to reveal the exact blast radius.
5. **Routes & Components**: Trace HTTP routes via `imc trace "GET /api/path"`, or inspect full component architecture via `imc context <file>`.

## What to run, when

| You want to | Run |
|---|---|
| get full task context in one shot | `imc pack "<query>"` |
| find code by concept or keyword | `imc search "<query>" --raw` |
| quick 1-line match preview | `imc search "<query>" --concise` |
| see file outline / list of methods | `imc read <file> --symbols` |
| see signatures & types without bodies (~90% token saving) | `imc read <file> --skeleton` |
| read one function/method/class body | `imc read <file> --method <name>` |
| see what calls it before you change it | `imc trace <symbol> --callers --depth 3` |
| see what it calls (execution flow) | `imc trace <symbol>` |
| find an HTTP route's backend handler | `imc trace "GET /api/path"` |
| see whole feature (deps, hooks, tests) | `imc context <file>` |
| recall a decision this repo already made | `imc memory-search "<topic>"` |
| retrieve full uncompressed tool output | `imc retrieve <id>` |

## The commands

**`imc pack "<query>"`** — assemble the context an agent needs in one command:
hybrid retrieval plus call-graph neighborhood plus project memories, budgeted into tokens.
Use this instead of stacking manual search, read, and context calls.
- `--budget 6000` — token budget for the whole bundle (default 3000).
- `--rerank` — use LLM to score and rerank candidate chunks before packing.
- `--no-memories` — leave memories out if only code is needed.
- `--json` — machine-readable bundle.

**`imc search "<query>"`** — hybrid retrieval (BM25 + vector) over codebase chunks.
- `--raw` — **Recommended for agents**: returns raw ranked chunks directly without LLM summarization (default budget 1500 tokens). Read the top 5, never stop at hit 1.
- `--rerank` — use LLM to score and rerank candidate chunks, outputting ranked code directly.
- `--concise` — one line per hit (~8x fewer tokens) to survey matches quickly.
- `--budget 1500` (or `-b`) — limit returned chunks by token budget to prevent context bloat.
- `-l 20` — how many chunks to retrieve (10 by default).
- `--offset N` — page through a long result set.
- `--no-vector` — BM25 only: no embedding call, offline and deterministic.
- `--json` — machine-readable hits.

**`imc read <file>`** — the token-optimised reader.
- `--skeleton` — the interface skeleton: types, signatures, and docstrings with function bodies folded to `{ ... }`. Use this first to grasp APIs without spending tokens on implementation details.
- `--symbols` — the outline. Start here.
- `--method <name>` — one symbol's body. **Use the real name from `--symbols`, including any leading underscore** (`_blended_score`, not `blended_score`).
- `--offset N -l 200` — a line range, for files without symbols or unindexed sections.
- `--json` — machine-readable.

**`imc trace <symbol>`** — the call tree, rooted at the definition line.
- `--callers --depth 3` — the blast radius. **Run this before you rename or delete anything.**
- `--both` — both directions (callees and callers).
- `imc trace "GET /api/path"` — an HTTP route to its backend handler.
- `-f <file>` — disambiguate when multiple files define the same symbol name.
- `--json` — machine-readable call tree.

**`imc context <file>`** — the whole feature around a file: component, deps,
endpoints, tests. `-d 2` resolves one more level of subcomponents.

**`imc memory-search "<topic>"`** — conventions and architectural decisions
already recorded for this repo.
- `--mem-type work_fact` — filter by memory type (`work_fact`, `work_task`, `work_method`, `business_rule`).
- `--scene <scene>` — filter by working scene.
- Empty is a valid answer: it means "no matching memory", not "this repo has no memories".

**`imc retrieve <id>`** — retrieve the original full text behind a compression handle.
When tool output is compressed with `[imc: ... full output is id <id>]`, run this to inspect the uncompressed original.

## Rules

1. **Search before you edit.** Never invent an API, path, or line number.
2. **Cite `path/to/file:L10-L25`** — and only a range you actually saw in an imc output.
3. **No blind whole-file reads.** Outline with `imc read <file> --symbols` first, then fetch functions with `imc read <file> --method <name>`.
4. **Check blast radius before edits.** Run `imc trace <symbol> --callers` before modifying shared functions.
5. **Ranked is not proven.** `imc trace` has no relevance floor: verify a hit before relying on it.
6. **Empty means "not in the index"**, not "does not exist". Fall back to Read/Grep.
7. **Use `-C <project-root>`** (or `-p <id>`) when the cwd may not be the project root.
8. Tool results are filtered before they reach you; large tool outputs carry retrieval handles (`imc retrieve <id>`).
9. **Run standalone single commands.** Never chain commands with `&&`, `;`, or pipe with `|` (`head`, `grep`). Compound shell commands trigger Auto Mode safety blocks. Use built-in imc flags: `-l <n>` for limits, `--depth <d>` for trace depth, `--budget <b>` for packing, and `-C <dir>` for directory targeting.
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

/// Ensure an entry exists in `.git/info/exclude` without duplicating lines.
pub fn ensure_git_exclude(project_root: &Path, pattern: &str) -> Result<()> {
    let git_path = project_root.join(".git");
    let info_dir = if git_path.is_dir() {
        Some(git_path.join("info"))
    } else if git_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            if let Some(gitdir) = content.trim().strip_prefix("gitdir:") {
                let p = PathBuf::from(gitdir.trim());
                let target = if p.is_absolute() { p } else { project_root.join(p) };
                Some(target.join("info"))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some(dir) = info_dir {
        std::fs::create_dir_all(&dir)?;
        let exclude_path = dir.join("exclude");
        let content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
        if !content.lines().any(|line| line.trim() == pattern.trim()) {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&exclude_path)?;
            if !content.is_empty() && !content.ends_with('\n') {
                writeln!(file)?;
            }
            writeln!(file, "{pattern}")?;
        }
    }
    Ok(())
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

    // Pre-approve imc CLI commands under Bash so Claude Code's Auto Mode
    // bypasses the classifier check and does not safety-block imc calls.
    let perms = obj
        .entry("permissions")
        .or_insert_with(|| json!({}));
    if !perms.is_object() {
        *perms = json!({});
    }
    let perms_obj = perms.as_object_mut().expect("permissions object");
    let allow = perms_obj
        .entry("allow")
        .or_insert_with(|| json!([]));
    if !allow.is_array() {
        *allow = json!([]);
    }
    if let Some(arr) = allow.as_array_mut() {
        let has_broad = arr.iter().any(|v| {
            v.as_str() == Some("Bash") || v.as_str() == Some("Bash(*)")
        });
        if !has_broad {
            for cmd in ["Bash(imc *)", "Bash(imc)"] {
                if !arr.iter().any(|v| v.as_str() == Some(cmd)) {
                    arr.push(json!(cmd));
                }
            }
        }
    }

    let mut text = serde_json::to_string_pretty(&data)?;
    text.push('\n');
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}


/// Remove the imc block an older `imc setup` wrote into CLAUDE.md.
///
/// The rule file is the rule now. A block left behind in CLAUDE.md is a
/// second copy of the same instructions, frozen at the moment it was written:
/// it drifts from the rule file and the agent reads the stale one.
pub fn remove_legacy_claude_md_block(project_root: &Path) -> Result<bool> {
    let path = project_root.join("CLAUDE.md");
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let (Some(b), Some(e)) = (existing.find(BEGIN_MARKER), existing.find(END_MARKER)) else {
        return Ok(false);
    };
    if b >= e {
        return Ok(false);
    }
    let mut updated = existing.clone();
    updated.replace_range(b..e + END_MARKER.len(), "");
    // Collapse the blank lines the removal leaves behind.
    let updated = updated
        .lines()
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
        + "\n";
    if updated == existing {
        return Ok(false);
    }
    std::fs::write(&path, updated).with_context(|| format!("write {}", path.display()))?;
    Ok(true)
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
    // A proxy that is already serving is the proxy. Starting a second one
    // on the same port would fail to bind and — worse — overwrite the
    // state file that says where the running one is.
    //
    // "Serving" means the process is alive, not that the port answers: a
    // socket in TIME_WAIT answers a connect after the process that owned it
    // is gone, and treating that as "a proxy is running" stops the real one
    // from ever being started.
    if let Some(status) = proxy_status(cfg) {
        if status.get("running").and_then(|v| v.as_bool()).unwrap_or(false) {
            let port_now = status
                .get("state")
                .and_then(|s| s.get("port"))
                .and_then(|v| v.as_u64())
                .unwrap_or(port as u64) as u16;
            let pid = status
                .get("state")
                .and_then(|s| s.get("pid"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if port_now == port && pid > 0 && process_alive(pid as u32) {
                return Ok(status.get("state").cloned().unwrap_or(json!({})));
            }
        }
    }
    // If the port is open but we reached here, an orphan or unmanaged process
    // is holding the port. Force-kill it before attempting to bind.
    if port_open(port) {
        let _ = stop_proxy(cfg);
        crate::web::kill_listener_on_port(port);
        std::thread::sleep(std::time::Duration::from_millis(200));
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
    let _ = &exe_disp;
    let state = write_proxy_state(cfg, port, pid)?;
    // The state file says the proxy is running; the port says whether it
    // is. A caller that starts the proxy and then talks to it needs the
    // second, so wait for it — a few hundred milliseconds at most.
    for _ in 0..50 {
        if port_open(port) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(state)
}

/// Record a running proxy in the state file.
///
/// The background starter and the foreground server both write it: a
/// proxy that is running is a proxy that `imc proxy status` should see and
/// `imc proxy stop` should be able to stop, no matter which way it was
/// started.
pub fn write_proxy_state(cfg: &Config, port: u16, pid: u32) -> Result<Value> {
    let exe = proxy_bin()?;
    let state = json!({
        "pid": pid,
        "port": port,
        "started_at": chrono_now(),
        "server_path": exe.display().to_string(),
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

/// Forget the proxy in the state file — it is no longer running.
///
/// Only when the state file still points at this process. A second proxy
/// that failed to bind the port (the first one is still there) must not
/// delete the record of the one that is actually serving: the state file
/// is the only record of where the proxy is, and losing it turns
/// `imc proxy stop` into "no proxy state file" while the proxy keeps
/// running.
pub fn clear_proxy_state(cfg: &Config) {
    let path = state_path(cfg);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let state: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
    let mine = state.get("pid").and_then(|v| v.as_u64());
    if mine == Some(std::process::id() as u64) {
        std::fs::remove_file(&path).ok();
    }
}

/// True when a process with this pid exists.
fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
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
    let mut stopped_port = None;
    if let Ok(raw) = std::fs::read_to_string(&p) {
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
        stopped_port = port;
        let _ = std::fs::remove_file(&p);
    }
    let target_port = stopped_port.unwrap_or(12345);
    if crate::web::kill_listener_on_port(target_port) {
        stopped_port = Some(target_port);
    }
    Ok(stopped_port)
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

/// Full setup: index → Claude settings → the rule file → start the proxy.
///
/// The proxy is optional: a machine that only needs the project wired (a
/// container, a CI step, a second checkout that talks to a proxy running
/// elsewhere) does not want a daemon it cannot see.
pub fn setup_and_start(
    cfg: &Config,
    root: &Path,
    port: u16,
    force_index: bool,
    start_the_proxy: bool,
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

    // 3. Claude Code settings + the rule file.
    //
    // The rule file is enough: it is the one the agent reads for this
    // project, and CLAUDE.md is the user's own file. Writing the block there
    // as well duplicated the same instructions in two places, and the two
    // copies drifted.
    let settings = write_claude_settings(root, port)?;
    log.push(format!("wrote {}", settings.display()));
    let rule = write_imc_rule(root)?;
    log.push(format!("wrote {}", rule.display()));
    // An older setup wrote a block into CLAUDE.md. That block is a stale
    // second copy of the rule; leaving it there means the agent reads the
    // old instructions.
    if remove_legacy_claude_md_block(root)? {
        log.push("removed the old imc block from CLAUDE.md".to_string());
    }

    // 4. Proxy.
    let key = cfg.llm_api_key();
    if key.is_empty() {
        log.push(format!(
            "WARNING: no upstream key — run `imc config set llm.api_key <key>` (see {})",
            imc_core::settings::path().display()
        ));
    }
    if start_the_proxy {
        let state = start_proxy(cfg, port)?;
        log.push(format!(
            "proxy running: pid={} http://127.0.0.1:{} → upstream {}",
            state["pid"], state["port"], state["upstream"]["base_url"]
        ));
    } else {
        log.push("proxy not started (--no-proxy)".to_string());
    }
    log.push("done — just run `claude` in this directory".to_string());
    Ok(log)
}

/// Ensure the project is indexed and registered as the active project.
pub fn ensure_project_indexed(cfg: &Config, root: &Path) -> Result<imc_index::IndexStats> {
    let printer = |_| {};
    let stats = imc_index::run_with_progress(cfg, root, &[], false, &printer)?;
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
    Ok(stats)
}

/// Run the `claude` binary on this machine, with all arguments passed
/// through.
///
/// Ensures the project is indexed, the proxy is running (so Claude Code talks to
/// it rather than to the upstream), wires the project's settings and rule file,
/// and then hands the terminal over to Claude Code.
pub fn run_claude_bin(cfg: &Config, root: &Path, args: &[String]) -> Result<()> {
    // 1. Ensure the project is indexed so code intelligence is ready.
    let _ = ensure_project_indexed(cfg, root);

    // 2. Ensure the proxy is running and fresh so Claude Code talks to our
    // compression proxy instead of hitting the upstream LLM directly.
    let port = 12345u16;
    let _ = start_proxy(cfg, port)
        .with_context(|| format!("start imc proxy on port {port}"))?;

    // The settings and the rule file are what make Claude Code use imc. A
    // project that was never wired would run Claude Code against the
    // upstream and know nothing about the index, so they are written here
    // too — wiring is cheap, and a stale rule is worse than a fresh one.
    let _ = write_claude_settings(root, 12345)?;
    let _ = write_imc_rule(root)?;
    let _ = remove_legacy_claude_md_block(root)?;

    let exe = which_claude_bin()?;
    let exe_disp = exe.display().to_string();

    // Claude Code is a terminal program. It needs the terminal: a child
    // whose output goes to a log file and whose input is closed is not a
    // session, it is a process nobody sees and nobody can type into. So the
    // standard streams are inherited, the working directory is the project
    // (so Claude Code reads that project's settings and rules), and the
    // parent waits for it — the exit code belongs to the caller.
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    // The proxy is where Claude Code sends its traffic. The settings file
    // says the same thing, but the environment says it for sure.
    let port = proxy_status(cfg)
        .and_then(|s| s.get("state").and_then(|st| st.get("port")).cloned())
        .and_then(|v| v.as_u64())
        .unwrap_or(12345) as u16;
    cmd.env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"));
    cmd.env("ANTHROPIC_AUTH_TOKEN", "dummy");
    cmd.env("ANTHROPIC_MODEL", "claude-pro-max");

    let status = cmd
        .status()
        .with_context(|| format!("run `{exe_disp}`"))?;
    if !status.success() {
        // The exit code belongs to the caller: `imc claude` is a wrapper
        // around `claude`, so `imc claude --print ...` fails the way
        // `claude --print ...` fails.
        if let Some(code) = status.code() {
            std::process::exit(code);
        }
        bail!("claude was killed by a signal");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_claude_settings_adds_permissions_allow() {
        let root = std::env::temp_dir().join(format!("imc-test-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let path = write_claude_settings(&root, 12345).unwrap();
        assert!(path.exists());

        let raw = std::fs::read_to_string(&path).unwrap();
        let val: Value = serde_json::from_str(&raw).unwrap();
        let allow = val["permissions"]["allow"].as_array().expect("allow array");
        assert!(allow.iter().any(|v| v.as_str() == Some("Bash(imc *)")));
        assert!(allow.iter().any(|v| v.as_str() == Some("Bash(imc)")));

        // Calling again does not duplicate entries
        let _ = write_claude_settings(&root, 12345).unwrap();
        let raw2 = std::fs::read_to_string(&path).unwrap();
        let val2: Value = serde_json::from_str(&raw2).unwrap();
        let allow2 = val2["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow2.iter().filter(|v| v.as_str() == Some("Bash(imc *)")).count(), 1);

        // Pre-existing broad "Bash" is not polluted with specific commands
        let broad_root = std::env::temp_dir().join(format!("imc-test-settings-broad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&broad_root);
        std::fs::create_dir_all(broad_root.join(".claude")).unwrap();
        let init_settings = json!({
            "permissions": { "allow": ["Bash"] }
        });
        std::fs::write(broad_root.join(".claude/settings.local.json"), serde_json::to_string_pretty(&init_settings).unwrap()).unwrap();
        let _ = write_claude_settings(&broad_root, 12345).unwrap();
        let broad_val: Value = serde_json::from_str(&std::fs::read_to_string(broad_root.join(".claude/settings.local.json")).unwrap()).unwrap();
        let broad_allow = broad_val["permissions"]["allow"].as_array().unwrap();
        assert_eq!(broad_allow.len(), 1);
        assert_eq!(broad_allow[0].as_str(), Some("Bash"));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&broad_root);
    }
}