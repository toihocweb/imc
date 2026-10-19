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
pub struct CompressionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_savings: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_tokens: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<IngestSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionSettings>,
    /// Pass-through for keys we do not model (jira, confluence, ...).
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_yaml::Value>,
}

pub fn path() -> PathBuf {
    std::env::var("IMC_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|h| h.join(".imc_config.yaml"))
                .unwrap_or_else(|| PathBuf::from(".imc_config.yaml"))
        })
}

/// Pre-rename location, still read so existing setups keep working.
pub fn legacy_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".indexer_config.yaml"))
        .unwrap_or_else(|| PathBuf::from(".indexer_config.yaml"))
}

/// Which file a `load` would actually read.
pub fn active_path() -> PathBuf {
    let p = path();
    if p.exists() || std::env::var("IMC_CONFIG").is_ok() {
        p
    } else {
        let legacy = legacy_path();
        if legacy.exists() {
            legacy
        } else {
            p
        }
    }
}

/// New file wins; otherwise fall back to the legacy file.
pub fn load() -> Settings {
    let p = path();
    if p.exists() {
        return load_at(&p);
    }
    let legacy = legacy_path();
    if legacy.exists() {
        return load_at(&legacy);
    }
    Settings::default()
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

/// Copy the legacy `~/.indexer_config.yaml` to `~/.imc_config.yaml` once.
pub fn migrate_legacy() -> Result<Option<PathBuf>> {
    let legacy = legacy_path();
    let new = path();
    if !legacy.exists() || new.exists() {
        return Ok(None);
    }
    std::fs::copy(&legacy, &new)
        .with_context(|| format!("copy {} -> {}", legacy.display(), new.display()))?;
    Ok(Some(legacy))
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
        ("compression", None) => serde_yaml::to_value(s.compression.as_ref()?).ok(),
        ("compression", Some("min_savings")) => s
            .compression
            .as_ref()?
            .min_savings
            .map(|v| serde_yaml::Value::Number(serde_yaml::Number::from(v))),
        ("compression", Some("min_tokens")) => s
            .compression
            .as_ref()?
            .min_tokens
            .map(|v| serde_yaml::Value::Number(serde_yaml::Number::from(v))),
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
        ("compression", Some("min_savings")) => {
            let v: f64 = value.parse().context("invalid min_savings value")?;
            s.compression.get_or_insert_default().min_savings = Some(v);
        }
        ("compression", Some("min_tokens")) => {
            let v: usize = value.parse().context("invalid min_tokens value")?;
            s.compression.get_or_insert_default().min_tokens = Some(v);
        }
        ("compression", None) => anyhow::bail!("use compression.min_savings | compression.min_tokens"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imc-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mask_handles_non_ascii_and_short_values() {
        let v = |s: &str| serde_yaml::Value::String(s.to_string());
        // Byte-indexed masking panicked on both of these.
        assert_eq!(mask(&v("갱신된토큰입니다요")), "갱신된토*입니다요");
        assert_eq!(mask(&v("갱신된토큰입니다")), "********");
        assert_eq!(mask(&v("abcd")), "****");
        assert_eq!(mask(&v("sk-abcdefghijklmnop")), "sk-a***********mnop");
    }

    #[test]
    fn set_and_get_roundtrip() {
        let dir = tmp("rt");
        let f = dir.join("imc_config.yaml");
        let mut s = Settings::default();
        set(&mut s, "llm.base_url", "http://localhost:11434/v1").unwrap();
        set(&mut s, "llm.api_key", "sk-test-1234567890abcdef").unwrap();
        set(&mut s, "embedding.model", "text-embedding-4-mini").unwrap();
        set(&mut s, "ingest.ignored_dirs", "a, b ,c").unwrap();
        save_at(&f, &s).unwrap();

        let loaded = load_at(&f);
        assert_eq!(
            get(&loaded, "llm.base_url").unwrap().as_str().unwrap(),
            "http://localhost:11434/v1"
        );
        assert_eq!(get(&loaded, "embedding.model").unwrap().as_str().unwrap(), "text-embedding-4-mini");
        let dirs = get(&loaded, "ingest.ignored_dirs").unwrap();
        assert_eq!(dirs.as_sequence().unwrap().len(), 3);
        // yaml shape matches the Python file
        let raw = std::fs::read_to_string(&f).unwrap();
        assert!(raw.contains("llm:"), "{raw}");
        assert!(raw.contains("embedding:"), "{raw}");
    }

    #[test]
    fn unknown_key_and_secrets_masked() {
        let mut s = Settings::default();
        assert!(set(&mut s, "llm.nope", "x").is_err());
        assert!(set(&mut s, "llm", "x").is_err());
        set(&mut s, "jira.token", "tok-abcdefgh").unwrap();
        let masked = mask(&get(&s, "jira.token").unwrap());
        assert!(masked.starts_with("tok-") && masked.contains('*'), "{masked}");
        assert!(!masked.contains("abcdefgh"), "secret leaked: {masked}");
    }

    #[test]
    fn save_is_atomic_and_overwrites() {
        let dir = tmp("atomic");
        let f = dir.join("imc_config.yaml");
        let mut s = Settings::default();
        set(&mut s, "llm.model", "m1").unwrap();
        save_at(&f, &s).unwrap();
        set(&mut s, "llm.model", "m2").unwrap();
        save_at(&f, &s).unwrap();
        assert_eq!(load_at(&f).llm.unwrap().model.unwrap(), "m2");
        assert!(!dir.join("imc_config.yaml.tmp").exists(), "tmp file left behind");
    }
}

/// `imc config list` output with the secret masked: `sk-...abcd`.
pub fn mask(v: &serde_yaml::Value) -> String {
    let s = match v {
        serde_yaml::Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    };
    // Count and cut by char: a non-ASCII value (or one whose 4th/last-4th byte
    // falls inside a character) panicked here, and this runs on every
    // `imc config` display.
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}{}{tail}", "*".repeat((chars.len() - 8).min(12)))
}
