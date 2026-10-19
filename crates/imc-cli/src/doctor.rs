//! `imc doctor` — diagnose environment, config, proxy, and project state,
//! then print concrete fix commands. Reduces manual troubleshooting.

use anyhow::{bail, Result};

use imc_core::config::Config;

/// Run the doctor check. Prints a human-readable report and exit code 0
/// if everything looks fine, non-zero if there are actionable issues.
pub fn run_doctor(cfg: &Config, root: &std::path::Path) -> Result<()> {
    let mut issues: Vec<String> = Vec::new();
    let mut ok: Vec<String> = Vec::new();

    // 1. Config file exists
    let config_path = imc_core::settings::path();
    if config_path.exists() {
        ok.push(format!("config file: {}", config_path.display()));
    } else {
        issues.push(format!(
            "config file missing: {} — run `imc config setup`",
            config_path.display()
        ));
    }

    // 2. LLM base_url configured
    let base_url = cfg.llm_base_url();
    if !base_url.is_empty() {
        ok.push(format!("llm.base_url: {}", base_url));
    } else {
        issues.push(
            "llm.base_url not set — run `imc config set llm.base_url <url>`".to_string(),
        );
    }

    // 3. LLM API key configured
    let api_key = cfg.llm_api_key();
    if !api_key.is_empty() {
        ok.push("llm.api_key: set (masked)".to_string());
    } else {
        issues.push(
            "llm.api_key not set — run `imc config set llm.api_key <key>`".to_string(),
        );
    }

    // 4. Embedding model configured
    if !cfg.embedding.model.is_empty() {
        ok.push(format!("embedding.model: {}", cfg.embedding.model));
    } else {
        issues.push(
            "embedding.model not set — run `imc config set embedding.model text-embedding-3-small`"
                .to_string(),
        );
    }

    // 5. Proxy running
    if super::claude::proxy_is_running(cfg) {
        ok.push("imc proxy: running on http://127.0.0.1:12345".to_string());
    } else {
        issues.push(
            "imc proxy not running — run `imc proxy serve --port 12345` or `imc claude setup`"
                .to_string(),
        );
    }

    // 6. Project indexed
    let db = cfg.project_db(root);
    if db.exists() {
        ok.push(format!("project indexed: {}", root.display()));
    } else {
        issues.push(format!(
            "project not indexed: {} — run `imc {}`",
            root.display(),
            root.display()
        ));
    }

    // 7. Claude settings present
    let claude_settings = root.join(".claude").join("settings.local.json");
    if claude_settings.exists() {
        ok.push("claude settings: .claude/settings.local.json exists".to_string());
    } else {
        issues.push(
            ".claude/settings.local.json missing — run `imc claude setup`".to_string(),
        );
    }

    // 8. imc rule file present
    let rule_file = root.join(".claude").join("rules").join("imc-rule.md");
    if rule_file.exists() {
        ok.push("imc rule: .claude/rules/imc-rule.md exists".to_string());
    } else {
        issues.push(
            ".claude/rules/imc-rule.md missing — run `imc claude setup`".to_string(),
        );
    }

    // Print report
    println!("=== imc doctor ===");
    println!();
    if !ok.is_empty() {
        println!("✓ OK:");
        for item in &ok {
            println!("  {}", item);
        }
        println!();
    }
    if !issues.is_empty() {
        println!("✗ Issues to fix:");
        for item in &issues {
            println!("  {}", item);
        }
        println!();
        println!("Run `imc config setup` for interactive configuration.");
        bail!("doctor found {} issue(s)", issues.len());
    } else {
        println!("Everything looks good. Run `imc claude setup` to wire Claude Code.");
        Ok(())
    }
}
