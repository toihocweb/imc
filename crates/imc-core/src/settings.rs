//! `~/.indexer_config.yaml` — the same config file the Python `main/core/config.py`
//! reads, so both versions share one source of truth.
//!
//! Shape (all sections optional):
//!   llm:        { base_url, api_key, model }
//!   embedding:  { model }
//!   ingest:     { ignored_dirs: [...] }

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmbeddingSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_dirs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<IngestSettings>,
    /// Pass-through for keys we do not model (jira, confluence, ...).
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_yaml::Value>,
}

pub fn path() -> PathBuf {
    std::env::var("IMC_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".indexer_config.yaml"))
                .unwrap_or_else(|| PathBuf::from(".indexer_config.yaml"))
        })
}

pub fn load() -> Settings {
    load_at(&path())
}

pub fn load_at(p: &std::path::Path) -> Settings {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_at(p: &std::path::Path, s: &Settings) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let yaml = serde_yaml::to_string(s).context("serialize settings")?;
    // Atomic replace: tmp file + rename, same discipline as Python's proxy writers.
    let tmp = p.with_extension("yaml.tmp");
    std::fs::write(&tmp, yaml.as_bytes()).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, p).with_context(|| format!("rename to {}", p.display()))?;
    Ok(())
}

pub fn save(s: &Settings) -> Result<()> {
    save_at(&path(), s)
}

/// Dotted get: `llm.base_url`, `embedding.model`, `ingest.ignored_dirs`.
pub fn get(s: &Settings, key: &str) -> Option<serde_yaml::Value> {
    let mut segs = key.splitn(2, '.');
    let head = segs.next()?;
    let tail = segs.next();
    match (head, tail) {
        ("llm", None) => serde_yaml::to_value(s.llm.as_ref()?).ok(),
        ("llm", Some(t)) => match t {
            "base_url" => s.llm.as_ref()?.base_url.clone().map(serde_yaml::Value::String),
            "api_key" => s.llm.as_ref()?.api_key.clone().map(serde_yaml::Value::String),
            "model" => s.llm.as_ref()?.model.clone().map(serde_yaml::Value::String),
            _ => None,
        },
        ("embedding", None) => serde_yaml::to_value(s.embedding.as_ref()?).ok(),
        ("embedding", Some("model")) => s.embedding.as_ref()?.model.clone().map(serde_yaml::Value::String),
        ("ingest", Some("ignored_dirs")) => s
            .ingest
            .as_ref()?
            .ignored_dirs
            .clone()
            .map(|v| {
                serde_yaml::Value::Sequence(v.into_iter().map(serde_yaml::Value::String).collect())
            }),
        _ => s.extra.get(head).cloned().map(|v| match tail {
            None => v,
            Some(t) => v.get(t).cloned().unwrap_or(serde_yaml::Value::Null),
        }),
    }
}

/// Dotted set: only `llm.*` and `embedding.model` are first-class; anything else
/// lands under `extra` so the Python side keeps reading it.
pub fn set(s: &mut Settings, key: &str, value: &str) -> Result<()> {
    let mut segs = key.splitn(2, '.');
    let head = segs.next().context("empty key")?.to_string();
    let tail = segs.next().map(str::to_string);
    match (head.as_str(), tail.as_deref()) {
        ("llm", Some("base_url")) => s.llm.get_or_insert_default().base_url = Some(value.into()),
        ("llm", Some("api_key")) => s.llm.get_or_insert_default().api_key = Some(value.into()),
        ("llm", Some("model")) => s.llm.get_or_insert_default().model = Some(value.into()),
        ("llm", Some(other)) => anyhow::bail!(
            "unknown llm key {other:?}; known: base_url, api_key, model"
        ),
        ("llm", None) => anyhow::bail!("use llm.base_url | llm.api_key | llm.model"),
        ("embedding", Some("model")) => {
            s.embedding.get_or_insert_default().model = Some(value.into())
        }
        ("embedding", None) => anyhow::bail!("use embedding.model"),
        ("ingest", Some("ignored_dirs")) => {
            let dirs: Vec<String> = value.split(',').map(|v| v.trim().to_string()).collect();
            s.ingest.get_or_insert_default().ignored_dirs = Some(dirs);
        }
        _ => {
            let v = match tail {
                Some(t) => {
                    let mut m = s.extra.remove(&head).and_then(|v| v.as_mapping().cloned());
                    let mut map = m.take().unwrap_or_else(serde_yaml::Mapping::new);
                    map.insert(
                        serde_yaml::Value::String(t),
                        serde_yaml::Value::String(value.into()),
                    );
                    serde_yaml::Value::Mapping(map)
                }
                None => serde_yaml::Value::String(value.into()),
            };
            s.extra.insert(head, v);
        }
    }
    Ok(())
}

/// `imc config list` output with the secret masked: `sk-...abcd`.
pub fn mask(v: &serde_yaml::Value) -> String {
    let s = match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    };
    if s.len() <= 8 {
        return "*".repeat(s.len());
    }
    format!("{}{}{}", &s[..4], "*".repeat((s.len() - 8).min(12)), &s[s.len() - 4..])
}
