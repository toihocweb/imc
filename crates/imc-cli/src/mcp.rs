//! MCP stdio server (JSON-RPC 2.0 over stdin/stdout).
//! Exposes the same tools as the Python `main/mcp/server.py`.

use std::io::{BufRead, Write};

use anyhow::Result;
use serde_json::{json, Value};

use imc_core::config::{load_registry, Config};
use imc_core::store;
use imc_search;

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tool_defs() -> Value {
    json!([
        {
            "name": "search_codebase",
            "description": "Hybrid search (BM25 + vectors + call-graph) over the indexed project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "default": 8}
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_project_graph",
            "description": "Definitions, callees and callers for a symbol name.",
            "inputSchema": {
                "type": "object",
                "properties": {"symbol": {"type": "string"}},
                "required": ["symbol"]
            }
        },
        {
            "name": "list_projects",
            "description": "List all indexed projects.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "index_stats",
            "description": "Counts of files, symbols, edges, chunks and embeddings.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

pub fn serve(cfg: &Config) -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":Value::Null,"error":{"code":-32700,"message":e.to_string()}});
                writeln!(out, "{err}")?;
                out.flush()?;
                continue;
            }
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "imc", "version": env!("CARGO_PKG_VERSION")}
            })),
            "tools/list" => Ok(json!({ "tools": tool_defs() })),
            "tools/call" => call_tool(cfg, &params),
            "ping" => Ok(json!({})),
            "notifications/initialized" => continue,
            other => {
                if id.is_none() {
                    continue;
                }
                Err((-32601, format!("method not found: {other}")))
            }
        };

        let payload = match result {
            Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        writeln!(out, "{payload}")?;
        out.flush()?;
    }
    Ok(())
}

fn text_result(body: String, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": body}],
        "isError": is_error
    })
}

fn call_tool(cfg: &Config, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let root = cfg
        .active_project()
        .ok_or((-32602, "no active project; run `imc index <path>`".to_string()))?;

    match name {
        "search_codebase" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(8) as usize;
            let store = store::open(&cfg.project_db(&root)).map_err(internal)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let hits = imc_search::search(&store, embedder.as_ref(), query, &imc_search::SearchOptions { limit: limit.clamp(1, 25), vector: true }).map_err(internal)?;
            let mut body = String::new();
            for (i, h) in hits.iter().enumerate() {
                body.push_str(&format!(
                    "{}. {}:{}-{} [{}] score={:.4}\n{}\n\n",
                    i + 1,
                    h.file,
                    h.start_line,
                    h.end_line,
                    h.sym_kind.as_deref().unwrap_or("chunk"),
                    h.score,
                    h.snippet
                ));
            }
            if body.is_empty() {
                body = format!("no matches for {query:?}");
            }
            Ok(text_result(body, false))
        }
        "get_project_graph" => {
            let symbol = args.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
            let store = store::open(&cfg.project_db(&root)).map_err(internal)?;
            let syms = store.find_symbols_by_name(symbol, 20);
            let mut body = String::new();
            for s in &syms {
                body.push_str(&format!(
                    "{} {} @ {}:{}-{}\n  calls: {}\n  called by: {}\n",
                    s.kind.as_str(),
                    s.qualname,
                    s.file,
                    s.line,
                    s.end_line,
                    store.callees(&s.id).join(", "),
                    store.callers(&s.name).join(", ")
                ));
            }
            if body.is_empty() {
                body = format!("symbol {symbol:?} not found");
            }
            Ok(text_result(body, false))
        }
        "list_projects" => {
            let projects = load_registry(cfg);
            let body = projects
                .iter()
                .map(|p| format!("{}  {}  ({} files)", p.id, p.path, p.file_count.unwrap_or(0)))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(text_result(body, false))
        }
        "index_stats" => {
            let store = store::open(&cfg.project_db(&root)).map_err(internal)?;
            let body = store
                .stats()
                .into_iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(text_result(body, false))
        }
        other => Err((-32602, format!("unknown tool: {other}"))),
    }
}

fn internal(e: impl std::fmt::Display) -> (i64, String) {
    (-32603, e.to_string())
}