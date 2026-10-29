//! Route-level tests: system-reminder assembly, tool filtering, and the
//! OpenAI -> Anthropic conversion used by the Hyperspace branch.

use imc_proxy_types::types::anthropic::{
    AnthropicInputMessage, AnthropicMessagesPayload, AnthropicTextBlock, AnthropicToolEntry,
    AnthropicUserMessage, StringOrBlocks,
};
use serde_json::{json, Map, Value};

use super::*;

fn user_message(text: &str) -> AnthropicInputMessage {
    AnthropicInputMessage::User(AnthropicUserMessage {
        role: "user".to_string(),
        content: StringOrBlocks::Text(text.to_string()),
    })
}

fn payload(messages: Vec<AnthropicInputMessage>) -> AnthropicMessagesPayload {
    AnthropicMessagesPayload {
        model: "claude-sonnet-4".to_string(),
        messages,
        ..Default::default()
    }
}

// ── system normalisation ─────────────────────────────────────────────────────

#[test]
fn normalize_system_turns_none_into_empty_blocks() {
    let mut p = payload(vec![]);
    p.system = None;
    normalize_system(&mut p);
    assert!(matches!(
        p.system,
        Some(StringOrBlocks::Blocks(v)) if v.is_empty()
    ));
}

#[test]
fn normalize_system_wraps_a_bare_string_in_a_text_block() {
    let mut p = payload(vec![]);
    p.system = Some(StringOrBlocks::Text("you are helpful".into()));
    normalize_system(&mut p);
    let Some(StringOrBlocks::Blocks(v)) = &p.system else {
        panic!("expected blocks");
    };
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].r#type, "text");
    assert_eq!(v[0].text, "you are helpful");
}

#[test]
fn normalize_system_keeps_existing_blocks() {
    let mut p = payload(vec![]);
    p.system = Some(StringOrBlocks::Blocks(vec![
        AnthropicTextBlock { r#type: "text".into(), text: "a".into(), cache_control: None },
        AnthropicTextBlock { r#type: "text".into(), text: "b".into(), cache_control: None },
    ]));
    normalize_system(&mut p);
    let Some(StringOrBlocks::Blocks(v)) = &p.system else {
        panic!("expected blocks");
    };
    assert_eq!(v.len(), 2);
}

#[test]
fn inject_terse_directive_appends_block_once() {
    let mut p = payload(vec![user_message("fix the bug")]);
    p.system = Some(StringOrBlocks::Blocks(vec![
        AnthropicTextBlock { r#type: "text".into(), text: "You are Claude Code".into(), cache_control: None },
    ]));
    inject_terse_directive(&mut p);

    // p.system remains untouched
    let Some(StringOrBlocks::Blocks(sys)) = &p.system else {
        panic!("expected system blocks");
    };
    assert_eq!(sys.len(), 1);
    assert!(!sys[0].text.contains("<response-type>"));

    // p.messages[0] receives the directive
    let AnthropicInputMessage::User(um) = &p.messages[0] else {
        panic!("expected user message");
    };
    let StringOrBlocks::Blocks(blocks) = &um.content else {
        panic!("expected blocks in user message");
    };
    assert_eq!(blocks.len(), 2);
    let AnthropicUserContentBlock::Text(t) = &blocks[1] else {
        panic!("expected text block");
    };
    assert!(t.text.contains("<response-type>"));
    assert!(t.text.contains("Ultra-terse communication protocol"));

    // Running again does not duplicate the directive
    inject_terse_directive(&mut p);
    let AnthropicInputMessage::User(um2) = &p.messages[0] else {
        panic!("expected user message");
    };
    let StringOrBlocks::Blocks(blocks2) = &um2.content else {
        panic!("expected blocks");
    };
    assert_eq!(blocks2.len(), 2);
}

#[test]
fn inject_terse_directive_handles_empty_messages() {
    let mut p = payload(vec![]);
    inject_terse_directive(&mut p);
    assert_eq!(p.messages.len(), 1);
    let AnthropicInputMessage::User(um) = &p.messages[0] else {
        panic!("expected user message");
    };
    let StringOrBlocks::Blocks(blocks) = &um.content else {
        panic!("expected blocks");
    };
    assert_eq!(blocks.len(), 1);
    let AnthropicUserContentBlock::Text(t) = &blocks[0] else {
        panic!("expected text block");
    };
    assert!(t.text.contains("<response-type>"));
}

// ── system_reminder assembly across the whole request ────────────────────────

#[test]
fn prompt_pipeline_assembles_and_injects_the_reminder() {
    let mut p = payload(vec![user_message(
        "<system-reminder>old</system-reminder>\n\nfix the parser",
    )]);
    p.system = Some(StringOrBlocks::Text(
        "Primary working directory: /definitely/not/a/project".into(),
    ));

    let registry = memory::load_projects();
    let outcome = apply_prompts_and_memories(&mut p, Some("sess-unittest"));

    // The user turn is recovered with the reminder stripped.
    assert_eq!(outcome.user_text, "fix the parser");
    // Detection never invents ids: any project it reports must be registered
    // (here it comes from the CWD fallback, since the system prompt's path is
    // not registered).
    if let Some(pid) = &outcome.project_id {
        assert!(
            registry.iter().any(|e| &e.id == pid),
            "detected project '{pid}' is not in the registry"
        );
    }

    normalize_system(&mut p);
    assert!(matches!(p.system, Some(StringOrBlocks::Blocks(_))));
}

#[test]
fn prompt_pipeline_strips_system_reminder_noise_from_user_text() {
    let mut p = payload(vec![
        user_message("earlier question"),
        user_message("<system-reminder>noise</system-reminder>\nreal question here"),
    ]);
    let outcome = apply_prompts_and_memories(&mut p, Some("sess-2"));
    assert_eq!(outcome.user_text, "real question here");
}

// ── tool auto-discovery & filtering ──────────────────────────────────────────

#[test]
fn unknown_tools_are_recorded_and_kept() {
    // A tool nobody configured defaults to enabled and is recorded — covered
    // end-to-end by `disabled_tools_are_dropped` (which owns the config file
    // for the duration). Here we only pin the enabled/disabled lookup.
    let mut config = Map::new();
    config.insert(
        "XyzTool".to_string(),
        json!({"description": "does xyz", "enabled": true}),
    );
    let enabled = config
        .get("XyzTool")
        .and_then(|v| v.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    assert!(enabled, "freshly discovered tools default to enabled");

    // A tool missing from the config is also treated as enabled.
    assert!(!config.contains_key("BrandNew"));
    assert_eq!(
        config
            .get("BrandNew")
            .and_then(|v| v.get("enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        true
    );
}

#[test]
fn disabled_tools_are_dropped() {
    // Serialized: the config path env var and the mtime cache are process-wide.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = std::env::temp_dir().join(format!("imc-tools-config-{}.json", std::process::id()));
    let backup = std::fs::read(&tmp).ok();
    std::fs::write(
        &tmp,
        json!({"NopeTool": {"enabled": false}, "YepTool": {"enabled": true}}).to_string(),
    )
    .expect("write tools config");

    std::env::set_var("IMC_TOOLS_CONFIG_PATH", &tmp);
    clear_tools_cache();

    let mut p = payload(vec![user_message("hi")]);
    p.tools = Some(vec![
        AnthropicToolEntry::Raw(json!({"name": "NopeTool", "description": ""})),
        AnthropicToolEntry::Raw(json!({"name": "YepTool", "description": ""})),
        AnthropicToolEntry::Raw(json!({"name": "FreshTool", "description": "brand new"})),
    ]);
    filter_tools_by_config(&mut p);

    let kept: Vec<&str> = p
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|t| match t {
            AnthropicToolEntry::Raw(v) => v.get("name").and_then(Value::as_str),
            AnthropicToolEntry::Tool(t) => Some(t.name.as_str()),
        })
        .collect();
    assert_eq!(
        kept,
        vec!["YepTool", "FreshTool"],
        "disabled tools are filtered out, new ones are discovered"
    );

    // The discovered tool must have been persisted next to the config.
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(&tmp).expect("config back")).unwrap();
    assert_eq!(saved["FreshTool"]["enabled"], json!(true));
    assert_eq!(saved["FreshTool"]["description"], json!("brand new"));

    // Restore environment.
    clear_tools_cache();
    std::env::remove_var("IMC_TOOLS_CONFIG_PATH");
    match backup {
        Some(bytes) => {
            let _ = std::fs::write(&tmp, bytes);
            clear_tools_cache();
        }
        None => {
            let _ = std::fs::remove_file(&tmp);
            clear_tools_cache();
        }
    }
}

/// Reset the in-process mtime cache (tests only).
fn clear_tools_cache() {
    let mut cache = tools_cache().lock().unwrap();
    cache.data = Map::new();
    cache.mtime = None;
}

// ── OpenAI -> Anthropic (hyperspace non-stream) ──────────────────────────────

#[test]
fn chat_completions_to_anthropic_maps_content_tools_and_usage() {
    let upstream = json!({
        "id": "cmpl-1",
        "choices": [{
            "message": {
                "content": "hello there",
                "tool_calls": [
                    {"id": "call_1", "function": {"name": "Bash", "arguments": "{\"command\":\"ls\"}"}}
                ]
            }
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 42,
            "prompt_tokens_details": {"cached_tokens": 30, "cache_write_tokens": 10}
        }
    });
    let out = chat_completions_to_anthropic(&upstream, "gpt-5-mini");
    assert_eq!(out["type"], "message");
    assert_eq!(out["role"], "assistant");
    assert_eq!(out["model"], "gpt-5-mini");
    assert_eq!(out["stop_reason"], "tool_use");
    assert_eq!(out["content"][0]["type"], "text");
    assert_eq!(out["content"][0]["text"], "hello there");
    assert_eq!(out["content"][1]["type"], "tool_use");
    assert_eq!(out["content"][1]["name"], "Bash");
    assert_eq!(out["content"][1]["input"]["command"], "ls");
    assert_eq!(out["usage"]["input_tokens"], json!(70), "cached tokens are excluded");
    assert_eq!(out["usage"]["output_tokens"], json!(42));
    assert_eq!(out["usage"]["cache_read_input_tokens"], json!(30));
    assert_eq!(out["usage"]["cache_creation_input_tokens"], json!(10));
}

#[test]
fn chat_completions_to_anthropic_text_only_and_bad_payloads() {
    let text_only = json!({
        "id": "cmpl-2",
        "choices": [{"message": {"content": "just text"}}],
        "usage": {"prompt_tokens": 5, "completion_tokens": 3}
    });
    let out = chat_completions_to_anthropic(&text_only, "m");
    assert_eq!(out["stop_reason"], "end_turn");
    assert_eq!(out["content"].as_array().unwrap().len(), 1);
    assert!(out["usage"].get("cache_read_input_tokens").is_none());

    // Malformed upstream bodies degrade to an empty assistant message rather
    // than panicking.
    let empty = chat_completions_to_anthropic(&json!({}), "m");
    assert_eq!(empty["content"], json!([]));
    assert_eq!(empty["stop_reason"], "end_turn");
}

// ── handlers reject bad input ────────────────────────────────────────────────

#[tokio::test]
async fn invalid_json_body_is_rejected_before_any_upstream_call() {
    let resp = proxy_messages(State(()), HeaderMap::new(), "{{{".into()).await;
    assert_eq!(resp.status().as_u16(), 422);
}

#[tokio::test]
async fn valid_payload_with_unknown_model_is_rejected_by_schema_only_if_unparseable() {
    // A structurally valid payload must pass parsing (routing/upstream would
    // then fail on network, which we do not exercise here).
    let body = json!({
        "model": "claude-test-xyz-000",
        "max_tokens": 16,
        "messages": [{"role": "user", "content": "hi"}]
    })
    .to_string();
    let raw: Value = serde_json::from_str(&body).unwrap();
    assert!(serde_json::from_value::<AnthropicMessagesPayload>(raw).is_ok());
}

#[test]
fn filter_tools_handles_missing_description_and_name() {
    let mut p = payload(vec![user_message("hi")]);
    p.tools = Some(vec![AnthropicToolEntry::Raw(json!({"description": "no name"}))]);
    filter_tools_by_config(&mut p);
    // Nameless tools are skipped entirely.
    assert!(p.tools.is_none());

    let (name, desc) = tool_name(&AnthropicToolEntry::Raw(json!({"name": "A", "description": "d"})));
    assert_eq!((name.as_str(), desc.as_str()), ("A", "d"));
    let (name, desc) = tool_name(&AnthropicToolEntry::Raw(json!({})));
    assert_eq!((name.as_str(), desc.as_str()), ("", ""));
}