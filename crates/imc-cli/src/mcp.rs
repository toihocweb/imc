//! `imc mcp` — MCP stdio server (Model Context Protocol).
//!
//! Exposes imc as an MCP server so any MCP client (Claude Code, Cursor,
//! Cline, …) can call imc tools through a real tool schema instead of
//! shelling out and guessing flags. Every tool delegates to
//! `imc -C <root> <verb> --json`, so the CLI contract stays single-sourced.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout. Messages
//! without an `id` are notifications and get no reply, per spec.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

/// The tool catalogue, the JSON-RPC dispatch and the tool implementations
/// live in `imc-mcp`, so the stdio transport (`imc mcp`) and the HTTP
/// transport (`POST /mcp` on the proxy) answer identically.
pub use imc_mcp::{call_tool, handle_jsonrpc, initialize, tools};

/// MCP project config written by `imc mcp install`.
pub const MCP_CONFIG_NAME: &str = ".mcp.json";

/// Tool catalogue advertised through `tools/list`.
pub fn serve(root: &Path) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let msg = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {"code": -32700, "message": "Parse error"}
                });
                let _ = writeln!(out, "{msg}");
                continue;
            }
        };

        // Notifications (no id) never get a response.
        let Some(resp) = imc_mcp::handle_jsonrpc(root, &req) else {
            continue;
        };
        let _ = writeln!(out, "{resp}");
    }
    Ok(())
}

pub fn install(root: &Path, config: Option<PathBuf>) -> Result<PathBuf> {
    let config_path = config.unwrap_or_else(|| root.join(MCP_CONFIG_NAME));

    let mut doc: Value = if config_path.is_file() {
        let s = fs::read_to_string(&config_path)?;
        serde_json::from_str(&s).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    if !doc.is_object() {
        doc = json!({});
    }
    if doc.get("mcpServers").is_none() {
        doc["mcpServers"] = json!({});
    }

    let bin = env::current_exe()?;
    doc["mcpServers"]["imc"] = json!({
        "command": bin.to_string_lossy(),
        "args": ["-C", root.to_string_lossy(), "mcp"],
    });

    fs::write(&config_path, serde_json::to_string_pretty(&doc)?)?;
    Ok(config_path)
}
