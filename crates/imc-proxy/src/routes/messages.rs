//! Port of `main/proxy/routes/messages.py` — the `/v1/messages` handler.
//!
//! This is the point of the proxy: it intercepts Claude Code's Anthropic
//! traffic, injects IMC search results as a `<system-reminder>`, filters noisy
//! tool results, and then relays to Hyperspace / Copilot / a configured
//! provider — streaming SSE without buffering the body.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use futures::StreamExt;
use imc_proxy_copilot::context_filters::{is_context_filters_enabled, process_context_filters};
use imc_proxy_copilot::create_messages::create_messages;
use imc_proxy_copilot::create_responses::create_responses;
use imc_proxy_copilot::preprocess::{
    apply_last_message_cache_control, apply_system_cache_control, apply_tools_cache_control,
};
use imc_proxy_copilot::responses_stream_translation::{
    translate_responses_stream_event, ResponsesStreamState,
};
use imc_proxy_copilot::responses_translation::{
    translate_responses_non_stream_result, translate_to_responses,
};
use imc_proxy_copilot::stream_translation::{
    format_sse_event, get_usage, translate_chunk_to_anthropic_events, AnthropicStreamState,
};
use imc_proxy_types::activity_extract::{
    publish_tool_results_from_payload, publish_tool_uses_from_content, StreamActivityExtractor,
};
use imc_proxy_types::atomic_file::write_file_atomically;
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::model_mapping::{apply_model_mapping, get_mapped_provider};
use imc_proxy_types::types::anthropic::{
    AnthropicMessagesPayload, AnthropicTextBlock, AnthropicToolEntry,
    StringOrBlocks,
};
use imc_proxy_types::types::chat_completions::ChatCompletionChunk;
use imc_proxy_types::usage_logger::log_usage;
use serde_json::{json, Map, Value};

use crate::github::token_provider;
use crate::hyperspace::call_hyperspace;
use crate::imc_memory::lib as memory;
use crate::providers::provider_proxy::{
    build_provider_upstream_headers, forward_provider_messages, resolve_provider,
    route_for_model, RouteTarget,
};

use super::error_response;

// ---------------------------------------------------------------------------
// Tool-management config (auto-discovery & filtering)
// ---------------------------------------------------------------------------

fn imc_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".indexmycode")
}

/// Port of `_TOOLS_CONFIG_PATH`. `IMC_TOOLS_CONFIG_PATH` overrides it (tests).
fn tools_config_path() -> PathBuf {
    if let Ok(custom) = std::env::var("IMC_TOOLS_CONFIG_PATH") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    imc_config_dir().join("tools_config.json")
}

#[derive(Default)]
struct ToolsConfigCache {
    data: Map<String, Value>,
    mtime: Option<std::time::SystemTime>,
}

fn tools_cache() -> &'static Mutex<ToolsConfigCache> {
    static CACHE: OnceLock<Mutex<ToolsConfigCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ToolsConfigCache::default()))
}

fn file_mtime(path: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Port of `_load_tools_config` — mtime-invalidated cache of
/// `~/.indexmycode/tools_config.json`.
pub fn load_tools_config() -> Map<String, Value> {
    let path = tools_config_path();
    let mtime = file_mtime(&path);

    {
        let cache = tools_cache().lock().expect("tools cache");
        if cache.mtime == mtime {
            return cache.data.clone();
        }
    }

    let mut data = Map::new();
    if mtime.is_some() {
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(Value::Object(obj)) => data = obj,
                Ok(_) => create_handler_logger("main")
                    .error("[Proxy] Error reading tools config: not an object"),
                Err(e) => create_handler_logger("main")
                    .error(&format!("[Proxy] Error reading tools config: {e}")),
            },
            Err(e) => create_handler_logger("main")
                .error(&format!("[Proxy] Error reading tools config: {e}")),
        }
    }

    let mut cache = tools_cache().lock().expect("tools cache");
    cache.data = data.clone();
    cache.mtime = mtime;
    data
}

/// Port of `_save_tools_config` — atomic tmp-file write + cache refresh.
pub fn save_tools_config(data: &Map<String, Value>) {
    let path = tools_config_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::create_dir_all(imc_config_dir()).is_err() {
        return;
    }
    let Ok(serialized) = serde_json::to_string_pretty(data) else {
        return;
    };
    if write_file_atomically(&tmp.to_string_lossy(), &serialized).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    // Python: `os.replace(tmp, _TOOLS_CONFIG_PATH)`.
    if let Err(e) = std::fs::rename(&tmp, &path) {
        create_handler_logger("main").error(&format!("[Proxy] Error writing tools config: {e}"));
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    let mut cache = tools_cache().lock().expect("tools cache");
    cache.data = data.clone();
    cache.mtime = file_mtime(&path);
}

fn tool_name(entry: &AnthropicToolEntry) -> (String, String) {
    match entry {
        AnthropicToolEntry::Tool(t) => (t.name.clone(), t.description.clone().unwrap_or_default()),
        AnthropicToolEntry::Raw(v) => (
            v.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            v.get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ),
    }
}

/// Port of the "TOOL MANAGEMENT (AUTO-DISCOVERY & FILTERING)" block.
///
/// Unknown tools are recorded as enabled; tools the user disabled in
/// `tools_config.json` are dropped from the request.
pub fn filter_tools_by_config(payload: &mut AnthropicMessagesPayload) {
    if !is_context_filters_enabled() {
        return;
    }
    let Some(tools) = payload.tools.as_ref() else {
        return;
    };
    if tools.is_empty() {
        return;
    }

    let mut config = load_tools_config();
    let mut config_updated = false;
    let mut filtered: Vec<AnthropicToolEntry> = Vec::new();

    for entry in tools.iter() {
        let (name, description) = tool_name(entry);
        if name.is_empty() {
            continue;
        }
        if !config.contains_key(&name) {
            config.insert(
                name.clone(),
                json!({"description": description, "enabled": true}),
            );
            config_updated = true;
        }
        let enabled = config
            .get(&name)
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if enabled {
            filtered.push(entry.clone());
        }
    }

    payload.tools = if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    };

    if config_updated {
        save_tools_config(&config);
    }
}

// ---------------------------------------------------------------------------
// system normalisation + reminder assembly
// ---------------------------------------------------------------------------

/// Port of the `payload.system` normalisation just before routing:
/// `None -> []`, `str -> [{type: text, text}]`.
pub fn normalize_system(payload: &mut AnthropicMessagesPayload) {
    match payload.system.take() {
        None => payload.system = Some(StringOrBlocks::Blocks(Vec::new())),
        Some(StringOrBlocks::Text(s)) => {
            payload.system = Some(StringOrBlocks::Blocks(vec![AnthropicTextBlock {
                r#type: "text".to_string(),
                text: s,
                cache_control: None,
            }]));
        }
        Some(StringOrBlocks::Blocks(blocks)) => payload.system = Some(StringOrBlocks::Blocks(blocks)),
    }
}

/// Port of the "DYNAMIC PROMPT TEMPLATES OVERRIDE" block: project detection,
/// then system-reminder template + learned-memory injection.
pub struct InjectionOutcome {
    pub project_id: Option<String>,
    pub user_text: String,
}

pub fn apply_prompts_and_memories(
    payload: &mut AnthropicMessagesPayload,
    session_id: Option<&str>,
) -> InjectionOutcome {
    let log = create_handler_logger("main");
    let projects = memory::load_projects();
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let cached = session_id.and_then(memory::get_session_project);

    let detected = memory::detect_project(payload, &projects, cached.as_deref(), cwd.as_deref());
    let project_id = detected.as_ref().map(|d| d.id.clone());
    if let (Some(detected), Some(sid)) = (detected.as_ref(), session_id) {
        memory::set_session_project(sid, &detected.id);
    }

    if let Some(pid) = project_id.as_deref() {
        log.info(&format!(
            "[Proxy] Applying prompt templates & learned memories for detected project: {pid}"
        ));
    }

    let user_text = memory::get_last_user_text(&payload.messages);
    memory::process_tool_outcomes(project_id.as_deref(), &payload.messages, session_id);
    memory::process_user_correction(project_id.as_deref(), Some(&user_text), session_id, &payload.messages);

    // Learn from the exchange, in the background.
    //
    // The proxy sees every correction and every failed command. When one
    // shows up, the exchange holds a lesson worth keeping. The call must not
    // touch the request: the gate is a few substring checks, the model call
    // runs on its own thread, and the request returns before any of it
    // starts.
    if let Some(detected) = detected.as_ref() {
        let answer = crate::imc_memory::learning::last_assistant_text(&payload.messages);
        let tool_output = crate::imc_memory::learning::tool_result_text(&payload.messages);
        if !answer.is_empty() && !user_text.is_empty() {
            crate::imc_memory::learning::learn_in_background(
                std::path::PathBuf::from(&detected.path),
                user_text.clone(),
                answer,
                tool_output,
            );
        }
    }

    // The system reminder is not injected any more.
    //
    // Claude Code loads the project's CLAUDE.md and its rule files itself,
    // and puts them in a `<system-reminder>` of its own. imc used to add
    // another one with the same text: the model read the same instructions
    // twice, and the prompt was longer for nothing. The project's own
    // instructions are the client's business.
    //
    // The memories are still injected — those are the ones the client
    // cannot know about.
    if memory::is_system_reminder_enabled(project_id.as_deref()) {
        memory::append_learned_memories_to_payload(payload, project_id.as_deref(), Some(&user_text), 4);
    }

    InjectionOutcome {
        project_id,
        user_text,
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Port of `proxy_messages`.
pub async fn proxy_messages(State(_state): State<()>, headers: HeaderMap, body: String) -> Response {
    let log = create_handler_logger("main");

    let raw: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return error_response(422, &format!("invalid JSON body: {e}")),
    };
    // Repair the tool ids on the raw body, before the typed parser sees it:
    // a block the typed structs do not recognise must not carry a broken id
    // upstream.
    let (raw, fixed_raw) = crate::sanitize::sanitize_tool_ids_in_json(raw);
    if fixed_raw > 0 {
        create_handler_logger("main").info(&format!(
            "[Proxy] Repaired {fixed_raw} invalid tool id(s) in the raw payload"
        ));
    }
    let mut payload: AnthropicMessagesPayload = match serde_json::from_value(raw.clone()) {
        Ok(p) => p,
        Err(e) => return error_response(422, &format!("invalid messages payload: {e}")),
    };

    // 0. Repair tool ids before anything else looks at them. Anthropic
    //    rejects the whole request with 400 when a `tool_use_id` does not
    //    match `^[a-zA-Z0-9_-]+$` (a provider id like `call_abc.def` is
    //    enough). The call and its result are rewritten with the same rule,
    //    so the pair survives.
    let ids = crate::sanitize::sanitize_tool_ids(&mut payload);
    if ids.fixed_ids > 0 {
        create_handler_logger("main").info(&format!(
            "[Proxy] Repaired {} invalid tool id(s) before forwarding",
            ids.fixed_ids
        ));
    }

    // 1. Model mapping + explicit provider prefix.
    let original_model = payload.model.clone();
    payload.model = apply_model_mapping(&payload.model);
    let explicit_provider = get_mapped_provider(&original_model);
    log.debug(&format!(
        "[Proxy] Mapped model '{original_model}' -> '{}' (provider: {}, stream={}, msgs={})",
        payload.model,
        explicit_provider.clone().unwrap_or_else(|| "auto".to_string()),
        payload.stream.unwrap_or(false),
        payload.messages.len()
    ));

    // 2. Log the user turn + publish tool results to the Activity panel.
    let session_id = memory::extract_session_id(&payload);
    let injection;

    let mut payload_for_activity = payload.clone();
    publish_tool_results_from_payload(&raw_messages_value(&payload_for_activity));
    payload_for_activity.messages.clear();
    payload_for_activity.messages.shrink_to_fit();

    let anthropic_beta = headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("prompt-caching-2024-07-31")
        .to_string();

    // 3. Tool management, context filters, prompt templates + memory injection.
    filter_tools_by_config(&mut payload);
    if is_context_filters_enabled() {
        // The learning pass needs the *original* tool results, so they are
        // collected before the filter runs. A result that no rule matched and
        // that is big enough to be worth a model call becomes a sample; the
        // model writes the rule, the rule is checked against the sample, and
        // from then on this command is filtered by it.
        let observations = if crate::context_learning::enabled() {
            imc_proxy_copilot::context_filters::collect_observations(&payload)
        } else {
            Vec::new()
        };
        process_context_filters(&mut payload);

        for obs in observations {
            if obs.matched_rule || obs.chars < crate::context_learning::MIN_LEARN_CHARS {
                continue;
            }
            if obs.command.trim().is_empty() {
                continue;
            }
            let observation = crate::context_learning::Observation {
                tool: obs.tool,
                command: obs.command,
                chars_before: obs.chars,
                sample: obs.sample,
            };
            // Off the request path: the model call must not delay the answer,
            // and the number of learning tasks is capped.
            crate::context_learning::learn_in_background(observation);
        }
    }

    // 3b. Shrink oversized tool results before they go upstream. The original
    // stays in the CCR store; the model gets a retrieval handle.
    let compression = crate::compression::context::compress_tool_results(
        &mut payload.messages,
        &crate::compression::context::ccr_db_path(),
    );
    if !compression.is_empty() {
        log.info(&format!(
            "[Compress] {} tool result(s): {} -> {} tokens (saved {})",
            compression.compressed,
            compression.original_tokens,
            compression.compressed_tokens,
            compression.saved_tokens()
        ));
    }

    injection = apply_prompts_and_memories(&mut payload, session_id.as_deref());
    if !injection.user_text.is_empty() {
        log.info(&format!("[USER] {}", injection.user_text));
    }
    normalize_system(&mut payload);

    // Prompt capture: store the payload exactly as the model will see it
    // (templates + injected memories included). Off unless
    // `imc capture on` (or the dashboard toggle) turned it on.
    if imc_core::capture::is_enabled() {
        if let Ok(v) = serde_json::to_value(&payload) {
            if let Some(path) = imc_core::capture::record_payload(&v) {
                log.debug(&format!("[Proxy] captured prompt -> {}", path.display()));
            }
        }
    }

    let model = payload.model.clone();
    let wants_stream = payload.stream.unwrap_or(false);
    let decision = route_for_model(
        &original_model,
        &model,
        explicit_provider.as_deref(),
    );
    log.info(&format!(
        "[Proxy] Route decision: hyperspace={}, model={}, starts_anthropic={}, explicit={}",
        decision.hyperspace,
        model,
        model.starts_with("anthropic--"),
        explicit_provider.clone().unwrap_or_else(|| "None".to_string())
    ));

    match decision.target {
        RouteTarget::ProviderPassthrough(provider_id) => {
            let Some(cfg) = resolve_provider(&provider_id) else {
                return error_response(502, &format!("provider '{provider_id}' is not configured"));
            };
            let mut request_headers = BTreeMap::new();
            for (name, value) in headers.iter() {
                if let Ok(v) = value.to_str() {
                    request_headers.insert(name.as_str().to_string(), v.to_string());
                }
            }
            let upstream_headers = build_provider_upstream_headers(&cfg, &request_headers);
            match forward_provider_messages(&cfg, &payload, &upstream_headers).await {
                Ok(resp) => return super::relay(resp, model, wants_stream, "/v1/messages").await,
                Err(e) => return error_response(502, &format!("{e:#}")),
            }
        }

        RouteTarget::Hyperspace => {
            log.info(&format!("[Hyperspace] Routing {model} to Hyperspace"));
            match call_hyperspace(&payload, Some(&anthropic_beta)).await {
                Ok(resp) if resp.status().as_u16() != 200 => {
                    let status = resp.status().as_u16();
                    let headers = crate::providers::provider_proxy::strip_response_headers(
                        resp.headers().clone(),
                    );
                    let bytes = resp.bytes().await.unwrap_or_default();
                    return build_response(status, headers, Body::from(bytes));
                }
                Ok(resp) => {
                    return hyperspace_response(resp, &payload, injection).await;
                }
                Err(e) => {
                    log.error(&format!("[Hyperspace] routing failed: {e:#}"));
                    return error_response(500, &format!("{e:#}"));
                }
            }
        }

        RouteTarget::CopilotResponses => {
            return responses_response(payload, injection).await;
        }

        RouteTarget::CopilotMessages => {
            return native_copilot_response(payload, headers, anthropic_beta, injection).await;
        }
    }
}

fn raw_messages_value(payload: &AnthropicMessagesPayload) -> Vec<Value> {
    payload
        .messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect()
}

fn build_response(status: u16, headers: HeaderMap, body: Body) -> Response {
    let mut response = Response::builder().status(status);
    for (name, value) in headers.iter() {
        response = response.header(name, value);
    }
    response.body(body).expect("response")
}

// ---------------------------------------------------------------------------
// Hyperspace: OpenAI SSE -> Anthropic SSE
// ---------------------------------------------------------------------------

async fn hyperspace_response(
    upstream: reqwest::Response,
    payload: &AnthropicMessagesPayload,
    injection: InjectionOutcome,
) -> Response {
    let log = create_handler_logger("main");
    let status = upstream.status().as_u16();
    let headers =
        crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());
    let model = payload.model.clone();

    if payload.stream.unwrap_or(false) {
        let mut stream = upstream.bytes_stream();
        let body_stream = async_stream::stream! {
            let mut state = AnthropicStreamState::default();
            let mut pending: Vec<u8> = Vec::new();
            let mut usage = Map::new();
            let mut answer_parts: Vec<String> = Vec::new();
            let mut activity = StreamActivityExtractor::new();

            while let Some(chunk) = stream.next().await {
                let Ok(chunk) = chunk else { break };
                pending.extend_from_slice(&chunk);
                // Only complete `data:` lines are consumed; anything partial
                // stays in `pending` for the next chunk.
                while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    let line = &line[..line.len() - 1];
                    let line = if line.last() == Some(&b'\r') { &line[..line.len() - 1] } else { line };
                    let Ok(line) = std::str::from_utf8(line) else { continue };
                    let Some(data_str) = line.strip_prefix("data: ") else { continue };
                    if data_str.trim() == "[DONE]" {
                        if let Some(pending_delta) = state.pending_message_delta.clone() {
                            let mut out = format_sse_event(&pending_delta).into_bytes();
                            out.push(b'\n');
                            yield Ok::<_, std::io::Error>(Bytes::from(out));
                        }
                        if !state.message_stop_sent {
                            yield Ok::<_, std::io::Error>(Bytes::from(
                                b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec(),
                            ));
                            state.message_stop_sent = true;
                        }
                        continue;
                    }
                    let Ok(chunk_obj) = serde_json::from_str::<ChatCompletionChunk>(data_str) else {
                        continue;
                    };
                    for event in translate_chunk_to_anthropic_events(&chunk_obj, &mut state) {
                        activity.feed_data_obj(&event);
                        if event.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                            if let Some(text) = event
                                .get("delta")
                                .and_then(|d| d.get("text"))
                                .and_then(Value::as_str)
                            {
                                answer_parts.push(text.to_string());
                            }
                        }
                        // Anthropic SSE needs the `event:` line — Claude Code's
                        // parser buffers the whole stream without it.
                        yield Ok::<_, std::io::Error>(Bytes::from(format_sse_event(&event)));
                    }
                    if chunk_obj.usage.is_some() {
                        if let Value::Object(u) = get_usage(&chunk_obj) {
                            usage = u;
                        }
                    }
                }
            }

            activity.flush();
            if !usage.is_empty() {
                let _ = log_usage(
                    &model,
                    "/v1/messages (hyperspace)",
                    &Value::Object(usage),
                    None,
                )
                .await;
            }
            if !answer_parts.is_empty() {
                log_answer(&log, &injection, &answer_parts.concat());
            }
        };
        return build_response(
            status,
            headers,
            Body::from_stream(body_stream),
        );
    }

    let bytes = upstream.bytes().await.unwrap_or_default();
    let Ok(data) = serde_json::from_slice::<Value>(&bytes) else {
        return build_response(status, headers, Body::from(bytes));
    };
    let anthropic = chat_completions_to_anthropic(&data, &model);
    if let Some(text) = anthropic
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
    {
        log_answer(&log, &injection, text);
    }
    if let Some(Value::Array(blocks)) = anthropic.get("content").cloned() {
        publish_tool_uses_from_content(&blocks);
    }
    build_response(status, headers, Body::from(anthropic.to_string()))
}

/// Port of the non-stream hyperspace block: OpenAI chat completion ->
/// Anthropic message.
pub fn chat_completions_to_anthropic(data: &Value, model: &str) -> Value {
    let message = data
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    let mut content_blocks: Vec<Value> = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            content_blocks.push(json!({"type": "text", "text": text}));
        }
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            let input = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .and_then(|a| serde_json::from_str::<Value>(a).ok())
                .unwrap_or_else(|| json!({}));
            content_blocks.push(json!({
                "type": "tool_use",
                "id": tc.get("id").cloned().unwrap_or(Value::String(String::new())),
                "name": tc.get("function").and_then(|f| f.get("name")).cloned().unwrap_or(Value::String(String::new())),
                "input": input,
            }));
        }
    }

    let stop_reason = if message.get("tool_calls").and_then(Value::as_array).is_some_and(|t| !t.is_empty()) {
        "tool_use"
    } else {
        "end_turn"
    };

    let mut usage_out = Map::new();
    if let Some(usage_in) = data.get("usage").filter(|u| !u.is_null()) {
        let prompt_tokens = usage_in.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
        let details = usage_in.get("prompt_tokens_details");
        let cached = details
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let cache_write = details
            .and_then(|d| {
                d.get("cache_write_tokens")
                    .or_else(|| d.get("cache_creation_tokens"))
            })
            .and_then(Value::as_i64)
            .unwrap_or(0);
        usage_out.insert(
            "input_tokens".to_string(),
            json!((prompt_tokens - cached).max(0)),
        );
        usage_out.insert(
            "output_tokens".to_string(),
            json!(usage_in.get("completion_tokens").and_then(Value::as_i64).unwrap_or(0)),
        );
        if cached > 0 {
            usage_out.insert("cache_read_input_tokens".to_string(), json!(cached));
        }
        if cache_write > 0 {
            usage_out.insert("cache_creation_input_tokens".to_string(), json!(cache_write));
        }
    }

    json!({
        "id": data.get("id").cloned().unwrap_or(Value::String(String::new())),
        "type": "message",
        "role": "assistant",
        "content": content_blocks,
        "model": model,
        "stop_reason": stop_reason,
        "usage": Value::Object(usage_out),
    })
}

// ---------------------------------------------------------------------------
// Copilot Responses API path
// ---------------------------------------------------------------------------

async fn responses_response(
    payload: AnthropicMessagesPayload,
    injection: InjectionOutcome,
) -> Response {
    let log = create_handler_logger("main");
    let model = payload.model.clone();
    let wants_stream = payload.stream.unwrap_or(false);
    let responses_payload = translate_to_responses(&payload);

    let upstream = match create_responses(&token_provider(), &responses_payload).await {
        Ok(r) => r,
        Err(e) => {
            log.error(&format!("[Proxy] Responses API setup exception: {e:#}"));
            return error_response(500, &format!("{e:#}"));
        }
    };
    if upstream.status().as_u16() != 200 {
        let status = upstream.status().as_u16();
        let headers =
            crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());
        let bytes = upstream.bytes().await.unwrap_or_default();
        return build_response(status, headers, Body::from(bytes));
    }
    let status = upstream.status().as_u16();
    let headers =
        crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());

    if wants_stream {
        let mut stream = upstream.bytes_stream();
        let body_stream = async_stream::stream! {
            let mut state = ResponsesStreamState::default();
            let mut pending: Vec<u8> = Vec::new();
            let mut usage = Map::new();
            let mut answer_parts: Vec<String> = Vec::new();
            while let Some(chunk) = stream.next().await {
                let Ok(chunk) = chunk else { break };
                pending.extend_from_slice(&chunk);
                while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = pending.drain(..=pos).collect();
                    line.pop();
                    if line.last() == Some(&b'\r') { line.pop(); }
                    let Ok(line) = std::str::from_utf8(&line) else { continue };
                    let Some(data_str) = line.strip_prefix("data: ") else { continue };
                    if data_str.trim() == "[DONE]" { continue; }
                    let Ok(data_obj) = serde_json::from_str::<Value>(data_str) else { continue };
                    if let Some(response_obj) = data_obj.get("response") {
                        if let Some(u) = response_obj.get("usage") {
                            let details = u.get("input_tokens_details");
                            usage = Map::from_iter([
                                ("input_tokens".to_string(), json!(u.get("input_tokens").cloned().unwrap_or(json!(0)))),
                                ("output_tokens".to_string(), json!(u.get("output_tokens").cloned().unwrap_or(json!(0)))),
                                ("cache_read_input_tokens".to_string(), json!(details.and_then(|d| d.get("cached_tokens")).cloned().unwrap_or(json!(0)))),
                                ("cache_creation_input_tokens".to_string(), json!(details.and_then(|d| d.get("cache_write_tokens")).cloned().unwrap_or(json!(0)))),
                            ]);
                        }
                    }
                    for event in translate_responses_stream_event(&data_obj, &mut state) {
                        if event.get("type").and_then(Value::as_str) == Some("content_block_delta") {
                            if let Some(text) = event.get("delta").and_then(|d| d.get("text")).and_then(Value::as_str) {
                                answer_parts.push(text.to_string());
                            }
                        }
                        yield Ok::<_, std::io::Error>(Bytes::from(format_sse_event(&event)));
                    }
                }
            }
            if !usage.is_empty() {
                let _ = log_usage(&model, "/v1/messages (translated responses)", &Value::Object(usage), None).await;
            }
            if !answer_parts.is_empty() {
                log_answer(&log, &injection, &answer_parts.concat());
            }
        };
        return build_response(status, headers, Body::from_stream(body_stream));
    }

    let bytes = upstream.bytes().await.unwrap_or_default();
    let Ok(result) = serde_json::from_slice::<Value>(&bytes) else {
        return build_response(status, headers, Body::from(bytes));
    };
    let anthropic_result = translate_responses_non_stream_result(&result);
    if let Some(text) = anthropic_result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|b| b.first())
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
    {
        log_answer(&log, &injection, text);
    }
    build_response(
        status,
        headers,
        Body::from(anthropic_result.to_string()),
    )
}

// ---------------------------------------------------------------------------
// Native Copilot path (raw Anthropic SSE passthrough)
// ---------------------------------------------------------------------------

async fn native_copilot_response(
    mut payload: AnthropicMessagesPayload,
    _request_headers: HeaderMap,
    anthropic_beta: String,
    injection: InjectionOutcome,
) -> Response {
    let log = create_handler_logger("main");
    let model = payload.model.clone();
    let wants_stream = payload.stream.unwrap_or(false);

    // Cache markers only on the native Anthropic path (Python: injected after
    // the hyperspace branch returns).
    apply_system_cache_control(&mut payload);
    apply_tools_cache_control(&mut payload);
    apply_last_message_cache_control(&mut payload);

    let upstream = match create_messages(&token_provider(), &payload, Some(&anthropic_beta)).await {
        Ok(r) => r,
        Err(e) => {
            log.error(&format!("[Proxy] Native Anthropic upstream error: {e:#}"));
            return error_response(500, &format!("{e:#}"));
        }
    };
    if upstream.status().as_u16() != 200 {
        let status = upstream.status().as_u16();
        let headers =
            crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());
        let bytes = upstream.bytes().await.unwrap_or_default();
        return build_response(status, headers, Body::from(bytes));
    }
    let status = upstream.status().as_u16();
    let headers =
        crate::providers::provider_proxy::strip_response_headers(upstream.headers().clone());

    if !wants_stream {
        let bytes = upstream.bytes().await.unwrap_or_default();
        if let Ok(data) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(usage) = data.get("usage") {
                let copilot_usage = data.get("copilot_usage").cloned();
                let model = model.clone();
                let usage = usage.clone();
                tokio::spawn(async move {
                    let _ = log_usage(
                        &model,
                        "/v1/messages",
                        &usage,
                        copilot_usage.as_ref(),
                    )
                    .await;
                });
            }
            if let Some(Value::Array(blocks)) = data.get("content").cloned() {
                publish_tool_uses_from_content(&blocks);
            }
            let mut answer = String::new();
            for block in data.get("content").and_then(Value::as_array).unwrap_or(&Vec::new()) {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    answer.push_str(block.get("text").and_then(Value::as_str).unwrap_or(""));
                }
            }
            if !answer.is_empty() {
                log_answer(&log, &injection, &answer);
            }
        }
        return build_response(status, headers, Body::from(bytes));
    }

    let mut stream = upstream.bytes_stream();
    let body_stream = async_stream::stream! {
        let mut pending: Vec<u8> = Vec::new();
        let mut usage = Map::new();
        let mut copilot_usage = Map::new();
        let mut answer_parts: Vec<String> = Vec::new();
        let mut activity = StreamActivityExtractor::new();

        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else { break };
            // Raw byte passthrough: upstream is already valid Anthropic SSE.
            yield Ok::<_, std::io::Error>(chunk.clone());
            // Side-buffer parsing; never gates the yield above.
            pending.extend_from_slice(&chunk);
            while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                let mut line: Vec<u8> = pending.drain(..=pos).collect();
                line.pop();
                if line.last() == Some(&b'\r') { line.pop(); }
                let Ok(line) = std::str::from_utf8(&line) else { continue };
                let Some(data_str) = line.strip_prefix("data: ") else { continue };
                if data_str.trim() == "[DONE]" { continue; }
                let Ok(data_obj) = serde_json::from_str::<Value>(data_str) else { continue };
                activity.feed_data_obj(&data_obj);
                match data_obj.get("type").and_then(Value::as_str) {
                    Some("message_start") => {
                        if let Some(Value::Object(u)) = data_obj.get("message").and_then(|m| m.get("usage")) {
                            usage = u.clone();
                        }
                    }
                    Some("message_delta") => {
                        if let Some(Value::Object(u)) = data_obj.get("usage") {
                            usage.extend(u.clone());
                        }
                        if let Some(Value::Object(c)) = data_obj.get("copilot_usage") {
                            copilot_usage.extend(c.clone());
                        }
                    }
                    Some("content_block_delta") => {
                        if data_obj.get("delta").and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta") {
                            answer_parts.push(
                                data_obj.get("delta").and_then(|d| d.get("text")).and_then(Value::as_str).unwrap_or("").to_string(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        activity.flush();
        if !usage.is_empty() {
            let _ = log_usage(
                &model,
                "/v1/messages",
                &Value::Object(usage),
                (!copilot_usage.is_empty()).then(|| Value::Object(copilot_usage)).as_ref(),
            ).await;
        }
        if !answer_parts.is_empty() {
            log_answer(&log, &injection, &answer_parts.concat());
        }
    };

    build_response(status, headers, Body::from_stream(body_stream))
}

fn log_answer(log: &imc_proxy_types::logger::Logger, injection: &InjectionOutcome, answer: &str) {
    log.info(&format!("[LLM] {answer}"));
    if let (Some(project_id), user_text) = (
        injection.project_id.as_deref(),
        injection.user_text.as_str(),
    ) {
        if !user_text.is_empty() {
            memory::log_interaction_for_project(Some(project_id), Some(user_text), Some(answer));
        }
    }
}

/// Routes served by this module.
pub fn router() -> Router {
    Router::new().route("/v1/messages", post(proxy_messages))
}

#[cfg(test)]
mod tests;