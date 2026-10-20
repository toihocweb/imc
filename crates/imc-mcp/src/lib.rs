//! The MCP server core, shared by the stdio transport (`imc mcp`) and the
//! HTTP transport (`POST /mcp` on the proxy).
//!
//! Every tool delegates to `imc -C <root> <verb> --json`, so the CLI contract
//! stays single-sourced.

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "search",
            "description": "Hybrid search over the project index (BM25 + graph + vector). Returns ranked hits with file, line range, score, symbol and source.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query (3-7 words)"},
                    "limit": {"type": "number", "description": "Max hits, default 10"},
                    "offset": {"type": "number", "description": "Skip this many hits to page through a result set"},
                    "no_vector": {"type": "boolean", "description": "Disable the vector leg (BM25 + graph only)"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "ask",
            "description": "LLM-synthesized answer over the index, with path:line citations. Requires llm.base_url in the imc config.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Question in natural language"},
                    "limit": {"type": "number", "description": "Hits fed to the LLM, default 5"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "trace",
            "description": "Call hierarchy for a symbol or HTTP route: callees (forward flow) or callers (reverse blast radius).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "Symbol name, file::qualified::name, or `GET /api/path` route"},
                    "callers": {"type": "boolean", "description": "Reverse trace: who calls this symbol"},
                    "depth": {"type": "number", "description": "Max tree depth, default 2"}
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "read",
            "description": "Token-optimised file reader: symbol outline, line ranges, single method.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": {"type": "string", "description": "Path to the file"},
                    "symbols": {"type": "boolean", "description": "Print the symbol outline only"},
                    "method": {"type": "string", "description": "Print one method/function/class by name"},
                    "offset": {"type": "number", "description": "Skip N lines from the top"},
                    "limit": {"type": "number", "description": "Max lines to print"}
                },
                "required": ["file"]
            }
        }),
        json!({
            "name": "context",
            "description": "One-shot resolved context for a file/module: outline, dependencies, related tests.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "File path or query target"},
                    "depth": {"type": "number", "description": "Subcomponent resolution depth, default 1"}
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "memory_search",
            "description": "Retrieve durable project memories (conventions, decisions, business rules) relevant to a query.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search query"},
                    "limit": {"type": "number", "description": "Max results, default 5"},
                    "mem_type": {"type": "string", "description": "work_fact|work_task|work_method|work_artifact|business_rule"},
                    "path": {"type": "string", "description": "File path the caller is looking at (adds an overlap boost)"}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "pack",
            "description": "Assemble an agent's context: retrieval + graph neighbourhood + project memories, compressed to a token budget.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What the agent wants to know or do"},
                    "budget": {"type": "number", "description": "Token budget for the whole bundle, default 6000"}
                },
                "required": ["query"]
            }
        }),
    ]
}

/// MCP server loop. Blocks until stdin closes.
fn build_argv(name: &str, a: &Value) -> Result<Vec<String>> {
    let mut v: Vec<String> = Vec::new();
    match name {
        "search" => {
            v.push("search".into());
            v.push(req_str(a, "query")?);
            v.push("--json".into());
            push_num(&mut v, a, "limit", "-l");
            push_num(&mut v, a, "offset", "--offset");
            if opt_bool(a, "no_vector") {
                v.push("--no-vector".into());
            }
        }
        "ask" => {
            v.push("search".into());
            v.push(req_str(a, "query")?);
            push_num(&mut v, a, "limit", "-l");
        }
        "trace" => {
            v.push("trace".into());
            v.push(req_str(a, "target")?);
            v.push("--json".into());
            if opt_bool(a, "callers") {
                v.push("--callers".into());
            }
            push_num(&mut v, a, "depth", "-d");
        }
        "read" => {
            v.push("read".into());
            v.push(req_str(a, "file")?);
            v.push("--json".into());
            if opt_bool(a, "symbols") {
                v.push("--symbols".into());
            }
            if let Some(m) = opt_str(a, "method") {
                v.push("--method".into());
                v.push(m);
            }
            push_num(&mut v, a, "offset", "--offset");
            push_num(&mut v, a, "limit", "-l");
        }
        "context" => {
            v.push("context".into());
            v.push(req_str(a, "target")?);
            v.push("--json".into());
            push_num(&mut v, a, "depth", "-d");
        }
        "memory_search" => {
            v.push("memory-search".into());
            v.push(req_str(a, "query")?);
            v.push("--json".into());
            push_num(&mut v, a, "limit", "-l");
            if let Some(t) = opt_str(a, "mem_type") {
                v.push("--mem-type".into());
                v.push(t);
            }
            if let Some(p) = opt_str(a, "path") {
                v.push("--path".into());
                v.push(p);
            }
        }
        "pack" => {
            v.push("pack".into());
            v.push(req_str(a, "query")?);
            v.push("--json".into());
            push_num(&mut v, a, "budget", "-b");
        }
        other => bail!("Unknown tool: {other}"),
    }
    Ok(v)
}

fn req_str(a: &Value, key: &str) -> Result<String> {
    opt_str(a, key).ok_or_else(|| anyhow::anyhow!("missing required argument `{key}`"))
}

fn opt_str(a: &Value, key: &str) -> Option<String> {
    a.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn opt_bool(a: &Value, key: &str) -> bool {
    a.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn push_num(v: &mut Vec<String>, a: &Value, key: &str, flag: &str) {
    if let Some(n) = a.get(key).and_then(Value::as_u64) {
        v.push(flag.to_string());
        v.push(n.to_string());
    }
}

/// Delegate to our own binary with the server's root pinned, so a call can
/// never fall through to whatever project happens to be globally active.
fn run_imc(root: &Path, argv: &[String]) -> Result<String> {
    let bin = env::current_exe()?;
    let out = Command::new(bin)
        .arg("-C")
        .arg(root)
        .args(argv)
        .stdin(Stdio::null())
        .output()?;

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if stderr.is_empty() {
            bail!("imc {} failed with {}", argv[0], out.status);
        }
        bail!("{stderr}");
    }
}

pub fn initialize(params: Option<&Value>) -> Value {
    let client_proto = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or("2024-11-05");
    json!({
        "protocolVersion": client_proto,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "imc", "version": env!("CARGO_PKG_VERSION") }
    })
}


/// A truncated result set must say so, otherwise the agent reads "10 hits"
/// as "there are 10 hits" and stops looking.
fn steer_note(name: &str, args: &Value, text: &str) -> Option<String> {
    if name != "search" {
        return None;
    }
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let parsed: Value = serde_json::from_str(text).ok()?;
    let arr = parsed.as_array()?;
    if arr.len() < limit {
        return None;
    }
    Some(format!(
        "_[more matches may exist: narrow the query, or page with `offset` (next page starts at {})]_",
        offset + arr.len()
    ))
}

/// `tools/call` always answers with a content array, even on failure, so the
/// client sees the error text instead of a transport error.
pub fn call_tool(root: &Path, params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    let argv = match build_argv(name, &args) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "content": [{"type": "text", "text": format!("{e:#}")}],
                "isError": true
            })
        }
    };

    match run_imc(root, &argv) {
        Ok(text) => {
            // The steer note goes into its own content block: the first block
            // stays pure JSON for anything that parses it.
            let mut content = vec![json!({"type": "text", "text": text})];
            if let Some(note) = steer_note(name, &args, &text) {
                content.push(json!({"type": "text", "text": note}));
            }
            json!({
                "content": content,
                "isError": false
            })
        }
        Err(e) => json!({
            "content": [{"type": "text", "text": format!("{e:#}")}],
            "isError": true
        }),
    }
}


/// Handle one JSON-RPC request.
///
/// Returns `None` for notifications (a message without an `id`), which get no
/// reply per the spec.
pub fn handle_jsonrpc(root: &Path, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params");

    let result = match method {
        "initialize" => Ok(initialize(params)),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => Ok(call_tool(root, params)),
        "ping" | "shutdown" => Ok(json!({})),
        other => Err(json!({
            "code": -32601,
            "message": format!("Method not found: {other}")
        })),
    };

    let Some(id) = id else { return None };
    Some(match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names() -> Vec<String> {
        tools()
            .iter()
            .map(|t| t["name"].as_str().unwrap_or("").to_string())
            .collect()
    }

    #[test]
    fn tools_cover_the_core_verbs() {
        let n = names();
        for want in ["search", "ask", "trace", "read", "context", "memory_search", "pack"] {
            assert!(n.contains(&want.to_string()), "missing tool {want}");
        }
    }

    #[test]
    fn every_tool_has_an_object_schema() {
        for t in tools() {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["description"].as_str().unwrap_or("").len() > 20);
        }
    }

    #[test]
    fn search_argv_pins_flags() {
        let argv = build_argv("search", &json!({"query": "auth flow", "limit": 3, "no_vector": true}))
            .unwrap();
        assert_eq!(argv, vec!["search", "auth flow", "--json", "-l", "3", "--no-vector"]);
    }

    #[test]
    fn trace_argv_handles_callers() {
        let argv = build_argv("trace", &json!({"target": "Foo.bar", "callers": true, "depth": 3})).unwrap();
        assert_eq!(argv, vec!["trace", "Foo.bar", "--json", "--callers", "-d", "3"]);
    }

    #[test]
    fn missing_required_argument_errors() {
        assert!(build_argv("search", &json!({})).is_err());
    }

    #[test]
    fn unknown_tool_errors() {
        assert!(build_argv("nope", &json!({})).is_err());
    }
}
