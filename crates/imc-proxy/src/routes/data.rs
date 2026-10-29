//! The dashboard's data API.
//!
//! The dashboard used to shell out to the `imc` binary for every request: one
//! subprocess per click, and a rebuild in progress was a dead page. The proxy
//! is already the process that holds the state, so the data is served from
//! here — the same surface the agents use, without a subprocess.
//!
//! Every handler is read-mostly and answers JSON. Secrets are masked exactly
//! as the CLI masks them: the browser never sees a key.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

/// The proxy's own config (the same one the CLI builds).
fn config() -> imc_core::config::Config {
    imc_core::config::Config::default()
}

/// The project a request is about. The dashboard sends the project's path; the
/// active project is the fallback.
fn project_root(cfg: &imc_core::config::Config, root: Option<&str>) -> PathBuf {
    match root.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => PathBuf::from(r),
        None => cfg
            .active_project()
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

fn error(message: impl Into<String>) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn as_bool(value: Option<&String>) -> bool {
    matches!(
        value.map(|v| v.as_str()),
        Some("1" | "true" | "yes" | "on")
    )
}

fn as_usize(value: Option<&String>, fallback: usize) -> usize {
    value.and_then(|v| v.parse().ok()).unwrap_or(fallback)
}

// ---------------------------------------------------------------------------
// GET /api/projects
// ---------------------------------------------------------------------------
async fn projects() -> Response {
    let cfg = config();
    let mut out = Vec::new();
    for p in imc_core::config::load_registry(&cfg) {
        let path = Path::new(&p.path);
        let mut entry = json!({ "id": p.id, "name": p.name, "path": p.path });
        let db = cfg.project_db(path);
        if let Ok(store) = imc_core::store::open(&db) {
            let mut counts = serde_json::Map::new();
            for (k, v) in store.stats() {
                counts.insert(k, json!(v));
            }
            entry["counts"] = Value::Object(counts);
            if let Ok(meta) = std::fs::metadata(&db) {
                entry["db_size_mb"] = json!(meta.len() as f64 / 1_048_576.0);
            }
        }
        out.push(entry);
    }
    Json(out).into_response()
}

// ---------------------------------------------------------------------------
// GET|POST /api/search
// ---------------------------------------------------------------------------
fn do_search(root: Option<&str>, query: &str, limit: usize, no_vector: bool) -> Response {
    if query.trim().is_empty() {
        return bad_request("query required");
    }
    let cfg = config();
    let root = project_root(&cfg, root);
    let db = cfg.project_db(&root);
    if !db.exists() {
        return error(format!(
            "project {} is not indexed — run `imc index {}` first",
            root.display(),
            root.display()
        ));
    }
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let embedder = imc_embed::build(&cfg.embedding);
    let opts = imc_search::SearchOptions {
        limit: limit.clamp(1, 50),
        vector: !no_vector,
    };
    match imc_search::search(&store, embedder.as_deref(), query, &opts) {
        Ok(hits) => Json(json!(hits)).into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

async fn search_get(Query(q): Query<HashMap<String, String>>) -> Response {
    let query = q.get("q").or_else(|| q.get("query")).cloned().unwrap_or_default();
    do_search(
        q.get("root").map(String::as_str),
        &query,
        as_usize(q.get("limit"), 10),
        as_bool(q.get("no_vector")),
    )
}

async fn search_post(Json(body): Json<Value>) -> Response {
    let query = body
        .get("query")
        .or_else(|| body.get("q"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    do_search(
        body.get("root").and_then(Value::as_str),
        &query,
        body.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize,
        body.get("noVector")
            .or_else(|| body.get("no_vector"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

// ---------------------------------------------------------------------------
// POST /api/trace
// ---------------------------------------------------------------------------
async fn trace(Json(body): Json<Value>) -> Response {
    let target = body
        .get("target")
        .or_else(|| body.get("symbol"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if target.trim().is_empty() {
        return bad_request("target required");
    }
    let cfg = config();
    let root = project_root(&cfg, body.get("root").and_then(Value::as_str));
    let db = cfg.project_db(&root);
    let indexed = db.exists();
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let bridge = imc_cli_extra::endpoint_bridge::EndpointRouteBridge::new(&root, Some(&store));
    let tracer =
        imc_cli_extra::adapter::ImcTracer::new(&store, &root, indexed).with_bridge(&bridge);

    let mut opts = imc_cli_extra::trace_cmd::TraceOptions::new(target);
    opts.depth = body
        .get("depth")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, 10) as usize;
    opts.json_output = true;
    opts.file_filter = body.get("file").and_then(Value::as_str).map(str::to_string);
    if body.get("both").and_then(Value::as_bool).unwrap_or(false) {
        opts.both = true;
    } else if body.get("callers").and_then(Value::as_bool).unwrap_or(false) {
        opts.callers = true;
    }

    match imc_cli_extra::trace_cmd::run_trace(&tracer, &opts) {
        Ok(out) => match out.json {
            Some(js) => match serde_json::from_str::<Value>(&js) {
                Ok(v) => Json(v).into_response(),
                Err(_) => Json(json!({ "raw": js })).into_response(),
            },
            None => Json(json!({ "text": out.text })).into_response(),
        },
        Err(e) => error(format!("{e:?}")),
    }
}

// ---------------------------------------------------------------------------
// GET|POST /api/memory
// ---------------------------------------------------------------------------
async fn memory_list(Query(q): Query<HashMap<String, String>>) -> Response {
    let cfg = config();
    let root = project_root(&cfg, q.get("root").map(String::as_str));
    let db = cfg.project_db(&root);
    if !db.exists() {
        return Json(json!([])).into_response();
    }
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let limit = as_usize(q.get("limit"), 500);
    let offset = as_usize(q.get("offset"), 0);
    match imc_memory::list_paged(&store, limit, offset) {
        Ok(items) => Json(json!(items)).into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

async fn memory_add(Json(body): Json<Value>) -> Response {
    let content = body
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if content.trim().is_empty() {
        return bad_request("content required");
    }
    let cfg = config();
    let root = project_root(&cfg, body.get("root").and_then(Value::as_str));
    let db = cfg.project_db(&root);
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let embedder = imc_embed::build(&cfg.embedding);
    let mem_type = body
        .get("mem_type")
        .or_else(|| body.get("memType"))
        .and_then(Value::as_str)
        .unwrap_or("note");
    let scene = body.get("scene").and_then(Value::as_str).unwrap_or("");
    let category = body
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or("note");
    let paths: Vec<String> = body
        .get("paths")
        .or_else(|| body.get("path"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = body
        .get("tags")
        .or_else(|| body.get("tag"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let importance = body
        .get("importance")
        .and_then(Value::as_f64)
        .unwrap_or(0.5);

    match imc_memory::add_typed(
        &store,
        &content,
        imc_memory::MemoryType::from_str(mem_type),
        scene,
        category,
        &paths,
        &tags,
        importance,
        "manual",
    ) {
        Ok(Some(id)) => {
            if let Err(e) = imc_memory::embed_and_store(&store, embedder.as_deref(), id, &content) {
                return error(format!("{e:#}"));
            }
            Json(json!({ "id": id, "stored": true })).into_response()
        }
        Ok(None) => Json(json!({ "id": Value::Null, "stored": false })).into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

// ---------------------------------------------------------------------------
// DELETE /api/memory  +  POST /api/memory/delete
// ---------------------------------------------------------------------------
fn do_memory_delete(root_str: Option<&str>, id_opt: Option<i64>) -> Response {
    let id = match id_opt {
        Some(i) => i,
        None => return bad_request("id required as integer"),
    };
    let cfg = config();
    let root = project_root(&cfg, root_str);
    let db = cfg.project_db(&root);
    if !db.exists() {
        return bad_request(format!("project database {} does not exist", db.display()));
    }
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    match imc_memory::delete(&store, id) {
        Ok(true) => Json(json!({ "ok": true, "deleted": true, "id": id })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "deleted": false, "error": format!("memory #{id} not found") })),
        )
            .into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

async fn memory_delete(Query(q): Query<HashMap<String, String>>) -> Response {
    let id = q.get("id").and_then(|v| v.parse::<i64>().ok());
    do_memory_delete(q.get("root").map(String::as_str), id)
}

async fn memory_delete_post(Json(body): Json<Value>) -> Response {
    let id = body
        .get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
    let root = body.get("root").and_then(Value::as_str);
    do_memory_delete(root, id)
}

// ---------------------------------------------------------------------------
// GET|POST /api/config
// ---------------------------------------------------------------------------
fn config_status() -> Value {
    use imc_core::settings as st;
    let s = st::load();
    let keys = [
        "llm.base_url",
        "llm.model",
        "llm.api_key",
        "embedding.model",
        "ingest.ignored_dirs",
    ];
    let mut raw = serde_json::Map::new();
    for key in keys {
        let value = st::get(&s, key);
        let masked = key.ends_with("api_key") || key.ends_with("token");
        let json_value = match value {
            Some(y) => {
                if masked {
                    json!("***")
                } else {
                    serde_json::to_value(
                        y.as_str().map(String::from).unwrap_or_else(|| format!("{y:?}")),
                    )
                    .unwrap_or(Value::Null)
                }
            }
            None => Value::Null,
        };
        raw.insert(key.to_string(), json_value);
    }
    let get = |key: &str| {
        raw.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let base_url = get("llm.base_url");
    let model = get("llm.model");
    let api_key = get("llm.api_key");
    let embedding_model = get("embedding.model");
    let mut missing = Vec::new();
    if base_url.is_empty() {
        missing.push("llm.base_url");
    }
    if model.is_empty() {
        missing.push("llm.model");
    }
    if api_key.is_empty() {
        missing.push("llm.api_key");
    }
    json!({
        "base_url": base_url,
        "model": model,
        "api_key_masked": api_key,
        "api_key_set": !api_key.is_empty(),
        "embedding_model": embedding_model,
        "config_path": imc_core::settings::active_path().display().to_string(),
        "complete": missing.is_empty(),
        "missing": missing,
        "settings": Value::Object(raw),
    })
}

async fn config_get() -> Response {
    Json(config_status()).into_response()
}

async fn config_set(Json(body): Json<Value>) -> Response {
    let key = body.get("key").and_then(Value::as_str).unwrap_or_default();
    let value = body.get("value").and_then(Value::as_str).unwrap_or_default();
    if key.trim().is_empty() {
        return bad_request("key required");
    }
    let mut s = imc_core::settings::load();
    match imc_core::settings::set(&mut s, key, value)
        .and_then(|_| imc_core::settings::save(&s))
    {
        Ok(()) => Json(json!({ "ok": true, "key": key })).into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

// ---------------------------------------------------------------------------
// GET /api/models
// ---------------------------------------------------------------------------
async fn models() -> Response {
    let cfg = config();
    let base = cfg.llm_base_url();
    if base.trim().is_empty() {
        return error("no llm.base_url configured — run `imc config set llm.base_url <url>`");
    }
    let key = cfg.llm_api_key();
    let url = format!("{}/models", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => return error(format!("{e}")),
    };
    let mut req = client.get(&url);
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return error(format!("{e}")),
    };
    let body = resp.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let mut entries: Vec<Value> = Vec::new();
    if let Some(arr) = parsed.get("data").and_then(Value::as_array) {
        entries.extend(arr.iter().cloned());
    } else if let Some(arr) = parsed.get("models").and_then(Value::as_array) {
        entries.extend(arr.iter().cloned());
    } else if let Some(arr) = parsed.as_array() {
        entries.extend(arr.iter().cloned());
    }
    let name_of = |v: &Value| -> String {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.get("id").and_then(Value::as_str).map(str::to_string))
            .or_else(|| v.get("name").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default()
    };
    let is_embedding = |v: &Value| -> bool {
        let lower = name_of(v).to_lowercase();
        lower.contains("embed") || lower.contains("bge-") || lower.contains("e5-")
    };
    let mut chat_models = Vec::new();
    let mut embedding_models = Vec::new();
    for entry in &entries {
        let name = name_of(entry);
        if name.is_empty() {
            continue;
        }
        if is_embedding(entry) {
            embedding_models.push(name);
        } else {
            chat_models.push(name);
        }
    }
    Json(json!({
        "url": url,
        "models": chat_models,
        "chat_models": chat_models,
        "embedding_models": embedding_models,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /api/compression/stats  +  GET /api/compression
// ---------------------------------------------------------------------------
async fn compression_stats() -> Response {
    let db = crate::compression::context::ccr_db_path();
    let store = match crate::compression::ccr_store::CcrStore::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let entries = match store.list(None, 1_000_000) {
        Ok(e) => e,
        Err(e) => return error(format!("{e:#}")),
    };
    let total_orig: i64 = entries.iter().map(|e| e.orig_tokens).sum();
    let total_comp: i64 = entries.iter().map(|e| e.comp_tokens).sum();
    let ratio = if total_orig > 0 {
        total_comp as f64 / total_orig as f64
    } else {
        1.0
    };
    let mut size = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    if entries.is_empty() && size > 65536 {
        let _ = store.conn.execute("VACUUM", []);
        size = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    }
    Json(json!({
        "db": db.display().to_string(),
        "records": entries.len(),
        "original_tokens": total_orig,
        "compressed_tokens": total_comp,
        "saved_tokens": total_orig - total_comp,
        "compression_ratio": ratio,
        "db_bytes": if entries.is_empty() { 0 } else { size },
    }))
    .into_response()
}

async fn compression_list(Query(q): Query<HashMap<String, String>>) -> Response {
    let db = crate::compression::context::ccr_db_path();
    let store = match crate::compression::ccr_store::CcrStore::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    if let Some(id) = q.get("id").filter(|s| !s.is_empty()) {
        let entry: Option<Value> = store
            .conn
            .query_row(
                "SELECT id, kind, original, compressed, orig_tokens, comp_tokens, created_at FROM ccr_originals WHERE id = ?1",
                rusqlite::params![id],
                |r| {
                    Ok(json!({
                        "id": r.get::<_, String>(0)?,
                        "kind": r.get::<_, String>(1)?,
                        "original": r.get::<_, String>(2)?,
                        "compressed": r.get::<_, String>(3)?,
                        "orig_tokens": r.get::<_, i64>(4)?,
                        "comp_tokens": r.get::<_, i64>(5)?,
                        "created_at": r.get::<_, i64>(6)?,
                    }))
                },
            )
            .ok();
        return match entry {
            Some(v) => Json(v).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("record {id} not found") })),
            )
                .into_response(),
        };
    }
    let limit = as_usize(q.get("limit"), 50);
    let entries = match store.list(None, limit) {
        Ok(e) => e,
        Err(e) => return error(format!("{e:#}")),
    };
    let out: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "kind": e.kind,
                "original": e.original,
                "compressed": e.compressed,
                "orig_tokens": e.orig_tokens,
                "comp_tokens": e.comp_tokens,
                "created_at": e.created_at,
            })
        })
        .collect();
    Json(json!(out)).into_response()
}

async fn compression_delete(Query(q): Query<HashMap<String, String>>) -> Response {
    let db = crate::compression::context::ccr_db_path();
    let mut store = match crate::compression::ccr_store::CcrStore::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    if let Some(id) = q.get("id") {
        return match store.delete(id) {
            Ok(n) => Json(json!({ "deleted": n })).into_response(),
            Err(e) => error(format!("{e:#}")),
        };
    }
    if let Some(days_str) = q.get("days") {
        if let Ok(days) = days_str.parse::<i64>() {
            return match store.prune_older_than_days(days) {
                Ok(n) => Json(json!({ "deleted": n })).into_response(),
                Err(e) => error(format!("{e:#}")),
            };
        }
    }
    if let Some(hours_str) = q.get("hours") {
        if let Ok(hours) = hours_str.parse::<i64>() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            return match store.prune_before(now - hours * 3600) {
                Ok(n) => Json(json!({ "deleted": n })).into_response(),
                Err(e) => error(format!("{e:#}")),
            };
        }
    }
    if as_bool(q.get("all")) {
        return match store.clear() {
            Ok(n) => Json(json!({ "deleted": n })).into_response(),
            Err(e) => error(format!("{e:#}")),
        };
    }
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "specify id, days, hours, or all=true" })),
    )
        .into_response()
}


// ---------------------------------------------------------------------------
// GET /api/config/test
// ---------------------------------------------------------------------------
async fn config_test() -> Response {
    let cfg = config();
    let base = cfg.llm_base_url();
    if base.trim().is_empty() {
        return Json(json!({ "ok": false, "output": "no llm.base_url configured" })).into_response();
    }
    let key = cfg.llm_api_key();
    let url = format!("{}/embeddings", base.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Json(json!({ "ok": false, "output": format!("{e}") })).into_response()
        }
    };
    let body = json!({ "model": cfg.embedding.model, "input": ["ping"] });
    let resp = client
        .post(&url)
        .header(
            "Authorization",
            format!("Bearer {}", if key.is_empty() { "dummy-key" } else { &key }),
        )
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            Json(json!({
                "ok": (200..300).contains(&status),
                "output": format!("OK  {url}  status={status}"),
            }))
            .into_response()
        }
        Err(e) => Json(json!({ "ok": false, "output": format!("FAIL  {url}\n  {e}") })).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/compress
// ---------------------------------------------------------------------------
async fn compress(Json(body): Json<Value>) -> Response {
    let text = body
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return bad_request("text required");
    }
    let result = crate::compression::compress(&text);
    Json(json!({
        "output": result.compressed,
        "kind": crate::compression::kind_slug(result.kind),
        "original_tokens": result.original_tokens,
        "compressed_tokens": result.compressed_tokens,
        "saved_tokens": result.saved_tokens,
        "compression_ratio": result.compression_ratio,
    }))
    .into_response()
}


// ---------------------------------------------------------------------------
// GET|POST /api/captures
// ---------------------------------------------------------------------------
async fn captures_get(Query(q): Query<HashMap<String, String>>) -> Response {
    let root = q.get("root").map(|s| Path::new(s.as_str()));
    if let Some(id) = q.get("id") {
        return match imc_core::capture::get_capture(id, root) {
            Some(payload) => Json(json!({ "ok": true, "payload": payload })).into_response(),
            None => error(format!("no capture {id}")),
        };
    }
    Json(json!({
        "ok": true,
        "status": imc_core::capture::status(root),
        "captures": imc_core::capture::list_captures(root),
    }))
    .into_response()
}

async fn captures_post(Json(body): Json<Value>) -> Response {
    let action = body.get("action").and_then(Value::as_str).unwrap_or("");
    let root_str = body.get("root").and_then(Value::as_str);
    let root = root_str.map(Path::new);
    let result = match action {
        "on" => imc_core::capture::set_enabled(true).map(|_| "capture on".to_string()),
        "off" => imc_core::capture::set_enabled(false).map(|_| "capture off".to_string()),
        "clear" => Ok(format!("cleared {}", imc_core::capture::clear_captures(root))),
        "delete" => match body.get("id").and_then(Value::as_str) {
            Some(id) => {
                if imc_core::capture::delete_capture(id, root) {
                    Ok(format!("deleted {id}"))
                } else {
                    Err(anyhow::anyhow!("no capture {id}"))
                }
            }
            None => Err(anyhow::anyhow!("id required")),
        },
        other => Err(anyhow::anyhow!("unknown action {other}")),
    };
    match result {
        Ok(message) => Json(json!({
            "ok": true,
            "message": message,
            "status": imc_core::capture::status(root),
            "captures": imc_core::capture::list_captures(root),
        }))
        .into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}


// ---------------------------------------------------------------------------
// GET /api/memory/stats
// ---------------------------------------------------------------------------
/// The state of the memory store.
///
/// The numbers a memory subsystem should be measured by: how much is stored,
/// how much of it is used, how much of it is unproven, and how fast it grows.
/// The contents never leave the machine.
async fn memory_stats(Query(q): Query<HashMap<String, String>>) -> Response {
    let cfg = config();
    let root = project_root(&cfg, q.get("root").map(String::as_str));
    let db = cfg.project_db(&root);
    if !db.exists() {
        return Json(json!({
            "project": root.display().to_string(),
            "total": 0, "active": 0, "pending": 0, "used": 0,
            "hit_rate": 0.0, "avg_importance": 0.0, "db_bytes": 0, "total_bytes": 0,
        }))
        .into_response();
    }
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => return error(format!("{e:#}")),
    };
    let all = imc_memory::list_all(&store, 100_000).unwrap_or_default();
    let total = all.len();
    let pending = all.iter().filter(|m| m.state == "pending").count();
    let used = all.iter().filter(|m| m.use_count > 0).count();
    let avg_importance = if total > 0 {
        all.iter().map(|m| m.importance).sum::<f64>() / total as f64
    } else {
        0.0
    };
    let total_bytes: usize = all.iter().map(|m| {
        m.content.len() + m.paths.iter().map(|p| p.len()).sum::<usize>() + m.tags.iter().map(|t| t.len()).sum::<usize>()
    }).sum();
    let db_bytes = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    Json(json!({
        "project": root.display().to_string(),
        "total": total,
        "active": total - pending,
        "pending": pending,
        "used": used,
        "hit_rate": if total > 0 { used as f64 / total as f64 } else { 0.0 },
        "avg_importance": avg_importance,
        "db_bytes": db_bytes,
        "total_bytes": total_bytes,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET|POST|DELETE /api/model-map
// ---------------------------------------------------------------------------
async fn model_map_get() -> Response {
    use imc_proxy_types::config_store as cs;
    let cfg = cs::read_config_from_disk();
    let mappings = cfg.model_mappings.unwrap_or_default();
    Json(json!({
        "path": cs::config_file_path().display().to_string(),
        "mappings": mappings,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct ModelMapSetPayload {
    from: String,
    to: String,
}

async fn model_map_post(Json(payload): Json<ModelMapSetPayload>) -> Response {
    use imc_proxy_types::config_store as cs;
    let from = payload.from.trim();
    let to = payload.to.trim();
    if from.is_empty() || to.is_empty() {
        return bad_request("from and to are required");
    }
    let mut cfg = cs::read_config_from_disk();
    let mut mappings = cfg.model_mappings.unwrap_or_default();
    mappings.insert(from.to_string(), to.to_string());
    cfg.model_mappings = Some(mappings);
    if let Err(e) = cs::write_config_to_disk(&cfg) {
        return error(format!("failed to write config: {e}"));
    }
    cs::invalidate_config_cache();
    model_map_get().await
}

#[derive(serde::Deserialize)]
struct ModelMapRmPayload {
    from: String,
}

async fn model_map_delete(Json(payload): Json<ModelMapRmPayload>) -> Response {
    use imc_proxy_types::config_store as cs;
    let from = payload.from.trim();
    if from.is_empty() {
        return bad_request("from is required");
    }
    let mut cfg = cs::read_config_from_disk();
    let mut mappings = cfg.model_mappings.unwrap_or_default();
    mappings.remove(from);
    cfg.model_mappings = Some(mappings);
    if let Err(e) = cs::write_config_to_disk(&cfg) {
        return error(format!("failed to write config: {e}"));
    }
    cs::invalidate_config_cache();
    model_map_get().await
}

// ---------------------------------------------------------------------------
// GET|DELETE /api/usage
// ---------------------------------------------------------------------------
async fn usage_get() -> Response {
    let file = imc_proxy_types::usage_logger::usage_log_file();
    let records = imc_proxy_types::usage_logger::read_records(&file);

    let mut total_input: i64 = 0;
    let mut total_output: i64 = 0;
    let mut total_cache_read: i64 = 0;
    let mut total_cache_creation: i64 = 0;

    #[derive(serde::Serialize, Default)]
    struct ModelStats {
        requests: usize,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
    }

    let mut by_model: BTreeMap<String, ModelStats> = BTreeMap::new();

    for r in &records {
        let model = r.get("model").and_then(Value::as_str).unwrap_or("unknown");
        let u = r.get("usage").unwrap_or(&Value::Null);
        let inp = u
            .get("input_tokens")
            .or_else(|| u.get("prompt_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let out = u
            .get("output_tokens")
            .or_else(|| u.get("completion_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cr = u.get("cache_read_input_tokens").and_then(Value::as_i64).unwrap_or(0);
        let cc = u.get("cache_creation_input_tokens").and_then(Value::as_i64).unwrap_or(0);

        total_input += inp;
        total_output += out;
        total_cache_read += cr;
        total_cache_creation += cc;

        let entry = by_model.entry(model.to_string()).or_default();
        entry.requests += 1;
        entry.input_tokens += inp;
        entry.output_tokens += out;
        entry.cache_read_tokens += cr;
    }

    let total_cacheable = total_input + total_cache_read + total_cache_creation;
    let hit_rate = if total_cacheable > 0 {
        total_cache_read as f64 / total_cacheable as f64
    } else {
        0.0
    };

    let count = records.len();
    let mut reversed = records;
    reversed.reverse();
    if reversed.len() > imc_proxy_types::usage_logger::MAX_USAGE_RECORDS {
        reversed.truncate(imc_proxy_types::usage_logger::MAX_USAGE_RECORDS);
    }

    Json(json!({
        "file": file.display().to_string(),
        "total_requests": count,
        "total_input_tokens": total_input,
        "total_output_tokens": total_output,
        "total_cache_read_tokens": total_cache_read,
        "total_cache_creation_tokens": total_cache_creation,
        "cache_hit_rate": hit_rate,
        "by_model": by_model,
        "records": reversed,
    }))
    .into_response()
}

async fn usage_delete() -> Response {
    let file = imc_proxy_types::usage_logger::usage_log_file();
    if let Err(e) = imc_proxy_types::atomic_file::write_file_atomically(&file.to_string_lossy(), "[]\n") {
        return error(format!("failed to clear usage: {e}"));
    }
    Json(json!({ "cleared": true })).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/connection/check
// ---------------------------------------------------------------------------
#[derive(serde::Deserialize)]
struct ConnectionCheckPayload {
    #[serde(default)]
    secret: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
}

async fn connection_check_get() -> Response {
    let s = imc_core::settings::load();
    let key = s
        .llm
        .as_ref()
        .and_then(|l| l.api_key.as_ref())
        .map(|k| k.trim().to_string())
        .unwrap_or_default();

    if key.is_empty() {
        return Json(json!({
            "ok": false,
            "configured": false,
            "error": "No API key configured in ~/.imc_config.yaml. Please enter your secret / API key.",
        }))
        .into_response();
    }

    let base = s
        .llm
        .as_ref()
        .and_then(|l| l.base_url.as_ref())
        .map(|b| b.trim_end_matches('/').to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:6655".to_string());

    probe_hyperspace(&key, &base, true).await
}

async fn connection_check(Json(body): Json<ConnectionCheckPayload>) -> Response {
    let key = body
        .secret
        .or(body.api_key)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    if key.is_empty() {
        return bad_request("Secret / API key is required");
    }

    let base = body
        .base_url
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:6655".to_string());

    probe_hyperspace(&key, &base, false).await
}

async fn probe_hyperspace(key: &str, base: &str, from_config: bool) -> Response {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return error(format!("Failed to build client: {e}")),
    };

    let probes = [
        format!("{base}/health"),
        format!("{base}/v1/models"),
        format!("{base}/models"),
        format!("{base}/anthropic/v1/models"),
        format!("{base}/"),
    ];

    let mut last_err = String::new();

    for url in &probes {
        let resp = client
            .get(url)
            .header("Authorization", format!("Bearer {key}"))
            .header("x-api-key", key)
            .send()
            .await;

        match resp {
            Ok(r) => {
                let status = r.status().as_u16();
                if status == 401 || status == 403 {
                    return Json(json!({
                        "ok": false,
                        "configured": true,
                        "status": status,
                        "error": format!("Hyperspace on port 6655 rejected secret (HTTP {status} Unauthorized). Please verify your API key."),
                    }))
                    .into_response();
                }
                if (200..300).contains(&status) {
                    if !from_config {
                        // Persist to imc settings
                        let mut s = imc_core::settings::load();
                        let _ = imc_core::settings::set(&mut s, "llm.api_key", key);
                        if s.llm
                            .as_ref()
                            .and_then(|l| l.base_url.as_ref())
                            .map(|b| b.is_empty())
                            .unwrap_or(true)
                        {
                            let _ = imc_core::settings::set(
                                &mut s,
                                "llm.base_url",
                                &format!("{base}/anthropic"),
                            );
                        }
                        let _ = imc_core::settings::save(&s);
                    }

                    return Json(json!({
                        "ok": true,
                        "configured": true,
                        "status": status,
                        "endpoint": url,
                        "message": if from_config {
                            "Hyperspace connection verified successfully via server configuration!"
                        } else {
                            "Hyperspace connection verified successfully!"
                        },
                    }))
                    .into_response();
                }
                last_err = format!("HTTP {status} from {url}");
            }
            Err(e) => {
                last_err = format!("{e}");
            }
        }
    }

    Json(json!({
        "ok": false,
        "configured": true,
        "error": format!("Could not connect to Hyperspace on port 6655: {last_err}. Please ensure Hyperspace is running on port 6655."),
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
pub fn router() -> Router {
    Router::new()
        .route("/api/projects", get(projects))
        .route("/api/search", get(search_get).post(search_post))
        .route("/api/trace", post(trace))
        .route("/api/memory", get(memory_list).post(memory_add).delete(memory_delete))
        .route("/api/memory/delete", post(memory_delete_post))
        .route("/api/memory/stats", get(memory_stats))
        .route("/api/config", get(config_get).post(config_set))
        .route("/api/config/test", get(config_test))
        .route("/api/models", get(models))
        .route("/api/model-map", get(model_map_get).post(model_map_post).delete(model_map_delete))
        .route("/api/compression/stats", get(compression_stats))
        .route("/api/compression", get(compression_list).delete(compression_delete))
        .route("/api/compress", post(compress))
        .route("/api/captures", get(captures_get).post(captures_post))
        .route("/api/usage", get(usage_get).delete(usage_delete))
        .route("/api/connection/check", post(connection_check).get(connection_check_get))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    #[test]
    fn as_bool_reads_the_usual_spellings() {
        assert!(as_bool(Some(&"true".to_string())));
        assert!(as_bool(Some(&"1".to_string())));
        assert!(!as_bool(Some(&"0".to_string())));
        assert!(!as_bool(None));
    }

    #[test]
    fn as_usize_falls_back() {
        assert_eq!(as_usize(Some(&"7".to_string()), 3), 7);
        assert_eq!(as_usize(Some(&"x".to_string()), 3), 3);
        assert_eq!(as_usize(None, 3), 3);
    }

    #[tokio::test]
    async fn projects_answer_an_array_even_with_no_registry() {
        let response = projects().await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_empty_query_is_a_bad_request() {
        let response = do_search(None, "  ", 10, false);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_config_answer_never_carries_a_key() {
        let status = config_status();
        let masked = status
            .get("api_key_masked")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert!(masked.is_empty() || masked == "***", "{masked}");
        if let Some(settings) = status.get("settings").and_then(Value::as_object) {
            for (key, value) in settings {
                if key.ends_with("api_key") || key.ends_with("token") {
                    assert_eq!(value.as_str(), Some("***"), "{key} leaked");
                }
            }
        }
    }

    #[tokio::test]
    async fn the_router_answers_the_dashboard_paths() {
        let app = router();
        for path in [
            "/api/projects",
            "/api/config",
            "/api/config/test",
            "/api/models",
            "/api/memory",
            "/api/captures",
            "/api/compression/stats",
            "/api/compression",
            "/api/usage",
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                response.status() != axum::http::StatusCode::NOT_FOUND,
                "{path} is not served"
            );
        }
    }

    #[tokio::test]
    async fn a_search_without_a_query_is_rejected() {
        let app = router();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/search")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn memory_delete_without_id_is_bad_request() {
        let app = router();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/api/memory")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compression_delete_without_params_is_bad_request() {
        let app = router();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/api/compression")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn connection_check_without_secret_is_bad_request() {
        let app = router();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/connection/check")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
