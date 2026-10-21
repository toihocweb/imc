//! The learning pass, end to end: a stub model answers with a rule, the rule
//! is checked against the sample it was written for, and it is stored where
//! the context filter will pick it up.


use axum::{routing::post, Json, Router};
use std::sync::Mutex;

/// The path override is an environment variable, so the tests that set it
/// have to run one after the other.
static ENV_LOCK: Mutex<()> = Mutex::new(());
use imc_proxy::context_learning::{self, Observation};
use serde_json::{json, Value};

/// A stub model: it answers every request with the same rule.
async fn stub_model(Json(body): Json<Value>) -> Json<Value> {
    let answer = r#"{"tool": "Bash", "command_pattern": "make build", "match_type": "startswith",
        "transforms": [{"type": "remove_lines", "pattern": "^progress \\d+"}]}"#;
    // Echo the model back so the test can assert the call used the config.
    let model = body
        .pointer("/model")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_string();
    Json(json!({
        "model": model,
        "choices": [{ "message": { "role": "assistant", "content": answer } }]
    }))
}

fn sample() -> String {
    let mut s = String::new();
    for i in 0..200 {
        s.push_str(&format!("progress {i} of 200\n"));
    }
    s.push_str("ERROR: the build failed at step 3\n");
    s
}

#[tokio::test]
async fn learns_a_rule_from_a_stub_model_and_stores_it() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // A stub model on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = Router::new().route("/v1/chat/completions", post(stub_model));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    // The learned rules go to a temp file, never to the real home.
    let dir = std::env::temp_dir().join(format!("imc-learn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("learned_filters.json");
    let _ = std::fs::remove_file(&path);
    // The path is read from the environment, so the test sets it before the
    // first call.
    std::env::set_var("IMC_LEARNED_FILTERS_PATH", &path);

    let settings = imc_core::settings::Settings {
        llm: Some(imc_core::settings::LlmSettings {
            base_url: Some(format!("http://{addr}/v1")),
            api_key: Some("test-key".to_string()),
            model: Some("stub-model".to_string()),
        }),
        ..Default::default()
    };

    let observation = Observation {
        tool: "Bash".to_string(),
        command: "make build".to_string(),
        chars_before: 4_000,
        sample: sample(),
    };

    let rule = context_learning::learn(observation, &settings)
        .await
        .expect("the rule must be learned");

    assert_eq!(rule["command_pattern"], json!("make build"));
    assert_eq!(rule["tool"], json!("Bash"));

    // It is on disk, in the format the context filter reads.
    let stored: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the file")).expect("JSON");
    let arr = stored.as_array().expect("an array");
    assert_eq!(arr.len(), 1, "{stored}");
    assert_eq!(arr[0]["source"], json!("learned"));
    assert_eq!(arr[0]["enabled"], json!(true));

    // And the context filter picks it up: the same command is filtered by it.
    let filters = imc_proxy_copilot::context_filters::load_tool_result_filters_config();
    assert!(
        filters.iter().any(|r| r.get("command_pattern") == Some(&json!("make build"))),
        "the learned rule must be in the filter list"
    );

    let filtered = imc_proxy_copilot::context_filters::filter_tool_result_content(
        &sample(),
        "make build",
        "Bash",
    );
    assert!(
        !filtered.contains("progress 5 of 200"),
        "the learned rule must drop the progress lines: {filtered}"
    );
    assert!(
        filtered.contains("ERROR: the build failed"),
        "the failure must survive: {filtered}"
    );

    server.abort();
    let _ = std::fs::remove_dir_all(&dir);
    std::env::remove_var("IMC_LEARNED_FILTERS_PATH");
}

/// A rule that would throw away the failure is rejected, so it is never
/// stored and never applied.
#[tokio::test]
async fn a_rule_that_drops_the_failure_is_never_stored() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("imc-learn-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let path = dir.join("learned_filters.json");
    let _ = std::fs::remove_file(&path);
    std::env::set_var("IMC_LEARNED_FILTERS_PATH", &path);

    let rule = json!({
        "tool": "Bash",
        "command_pattern": "make build",
        "match_type": "startswith",
        "transforms": [{ "type": "remove_lines", "pattern": ".*" }],
    });
    let err = context_learning::validate_rule(&rule, &sample()).expect_err("must be rejected");
    assert!(
        err.contains("failure line") || err.contains("empties the output"),
        "{err}"
    );
    assert!(!path.exists(), "nothing may be stored");

    std::env::remove_var("IMC_LEARNED_FILTERS_PATH");
    let _ = std::fs::remove_dir_all(&dir);
}

