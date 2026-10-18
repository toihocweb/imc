//! Port of `main/proxy/lib/models.py` — Claude model-id normalization helpers.

use crate::state::state;
use crate::types::models::Model;

/// Python returns `{"family": .., "version": ..}`; typed here for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedSdkModelId {
    pub family: String,
    pub version: String,
}

/// Port of `models.normalize_sdk_model_id`.
pub fn normalize_sdk_model_id(sdk_model_id: &str) -> Option<NormalizedSdkModelId> {
    if sdk_model_id.is_empty() {
        return None;
    }
    let lower = sdk_model_id.to_lowercase();
    let without_date = strip_trailing_date(&lower);

    // Pattern 1: claude-{family}-{major}.{minor}
    if let Some(c) = regex_captures(r"^claude-(\w+)-(\d+)\.(\d+)$", &without_date) {
        return Some(NormalizedSdkModelId {
            family: c[0].clone(),
            version: format!("{}.{}", c[1], c[2]),
        });
    }
    // Pattern 2: claude-{family}-{major}-{minor}
    if let Some(c) = regex_captures(r"^claude-(\w+)-(\d+)-(\d+)$", &without_date) {
        return Some(NormalizedSdkModelId {
            family: c[0].clone(),
            version: format!("{}.{}", c[1], c[2]),
        });
    }
    // Pattern 3: claude-{major}-{minor}-{family}
    if let Some(c) = regex_captures(r"^claude-(\d+)-(\d+)-(\w+)$", &without_date) {
        return Some(NormalizedSdkModelId {
            family: c[2].clone(),
            version: format!("{}.{}", c[0], c[1]),
        });
    }
    // Pattern 4: claude-{family}-{major}
    if let Some(c) = regex_captures(r"^claude-(\w+)-(\d+)$", &without_date) {
        return Some(NormalizedSdkModelId {
            family: c[0].clone(),
            version: c[1].clone(),
        });
    }
    // Pattern 5: claude-{major}-{family}
    if let Some(c) = regex_captures(r"^claude-(\d+)-(\w+)$", &without_date) {
        return Some(NormalizedSdkModelId {
            family: c[1].clone(),
            version: c[0].clone(),
        });
    }
    None
}

fn strip_trailing_date(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 9 {
        let tail = &s[s.len() - 8..];
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return s[..s.len() - 8].to_string();
        }
    }
    s.to_string()
}

/// Returns the regex capture groups as owned strings (avoids tying the caller's
/// borrow to a `'static` Regex).
fn regex_captures(pattern: &str, text: &str) -> Option<Vec<String>> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::collections::HashMap<&'static str, regex::Regex>> = OnceLock::new();
    let re = CACHE
        .get_or_init(|| {
            [
                r"^claude-(\w+)-(\d+)\.(\d+)$",
                r"^claude-(\w+)-(\d+)-(\d+)$",
                r"^claude-(\d+)-(\d+)-(\w+)$",
                r"^claude-(\w+)-(\d+)$",
                r"^claude-(\d+)-(\w+)$",
            ]
            .iter()
            .map(|p| (*p, regex::Regex::new(p).unwrap()))
            .collect()
        })
        .get(pattern)?;
    let caps = re.captures(text)?;
    Some(
        caps.iter()
            .skip(1)
            .map(|m| m.map(|s| s.as_str().to_string()).unwrap_or_default())
            .collect(),
    )
}

/// Port of `models.to_client_model_id`.
pub fn to_client_model_id(model_id: &str) -> String {
    let Some(normalized) = normalize_sdk_model_id(model_id) else {
        return model_id.to_string();
    };
    let version_hyphenated = normalized.version.replace('.', "-");
    format!("claude-{}-{version_hyphenated}", normalized.family)
}

fn models_data() -> Vec<Model> {
    state()
        .models
        .as_ref()
        .map(|m| m.data.clone())
        .unwrap_or_default()
}

/// Port of `models.find_endpoint_model` — looks up by raw id, then by the
/// normalized `claude-{family}-{v1}.{v2}` form.
pub fn find_endpoint_model(sdk_model_id: &str) -> Option<Model> {
    let data = models_data();
    for m in &data {
        if m.id == sdk_model_id {
            return Some(m.clone());
        }
    }
    let normalized = normalize_sdk_model_id(sdk_model_id)?;
    let model_name = format!("claude-{}-{}", normalized.family, normalized.version);
    data.into_iter().find(|m| m.id == model_name)
}

/// Port of `models.get_latest_model_for_family` — highest major.minor wins.
pub fn get_latest_model_for_family(family: &str) -> Option<Model> {
    let data = models_data();
    let mut best: Option<(Model, i64, i64)> = None;
    for model in data {
        let Some(normalized) = normalize_sdk_model_id(&model.id) else {
            continue;
        };
        if normalized.family != family {
            continue;
        }
        let mut parts = normalized.version.splitn(2, '.');
        let major_part = parts.next().unwrap_or("");
        let minor_part = parts.next().unwrap_or("0");
        let Ok(major) = major_part.parse::<i64>() else {
            continue;
        };
        let Ok(minor) = minor_part.parse::<i64>() else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((_, bm, bn)) => major > *bm || (major == *bm && minor > *bn),
        };
        if better {
            best = Some((model, major, minor));
        }
    }
    best.map(|(model, _, _)| model)
}