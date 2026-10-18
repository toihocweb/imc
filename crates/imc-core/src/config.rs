use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Embedding provider settings (OpenAI-compatible `/embeddings` endpoint).
impl EmbedConfig {
    /// Env vars win, then `~/.indexer_config.yaml` (llm.* / embedding.model).
    pub fn from_settings(s: &crate::settings::Settings) -> Self {
        let mut c = EmbedConfig::default();
        if c.api_key.is_none() {
            if let Some(k) = s.llm.as_ref().and_then(|l| l.api_key.as_ref()) {
                if !k.is_empty() {
                    c.api_key = Some(k.clone());
                }
            }
        }
        if let Some(b) = s.llm.as_ref().and_then(|l| l.base_url.as_ref()) {
            if std::env::var("IMC_EMBED_BASE_URL").is_err()
                && std::env::var("OPENAI_BASE_URL").is_err()
            {
                c.base_url = b.clone();
            }
        }
        if let Some(m) = s.embedding.as_ref().and_then(|e| e.model.as_ref()) {
            if std::env::var("IMC_EMBED_MODEL").is_err() {
                c.model = m.clone();
            }
        }
        c
    }
}

#[derive(Debug, Clone)]
pub struct EmbedConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Dimension used by the offline hashing fallback embedder.
    pub fallback_dim: usize,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            base_url: std::env::var("IMC_EMBED_BASE_URL")
                .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            api_key: std::env::var("IMC_EMBED_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .ok(),
            model: std::env::var("IMC_EMBED_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
            fallback_dim: 384,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub embedding: EmbedConfig,
    /// Files larger than this are skipped (1 MiB, matching the Python port).
    pub max_file_bytes: u64,
    /// Hard cap for text chunks used when a file has no AST symbols.
    pub window_lines: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(
                std::env::var("IMC_DATA_DIR").unwrap_or_else(|_| {
                    dirs::home_dir()
                        .map(|h| h.join(".indexmycode-rs").to_string_lossy().into_owned())
                        .unwrap_or_else(|| ".indexmycode-rs".to_string())
                }),
            ),
            embedding: EmbedConfig::from_settings(&crate::settings::load()),
            max_file_bytes: std::env::var("IMC_MAX_FILE_BYTES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1024 * 1024),
            window_lines: 120,
        }
    }
}

impl Config {
    /// `llm.base_url` from `~/.indexer_config.yaml` (default matches Python).
    pub fn llm_base_url(&self) -> String {
        crate::settings::load()
            .llm
            .and_then(|l| l.base_url)
            .unwrap_or_else(|| "http://localhost:6655/litellm/v1".to_string())
    }

    pub fn llm_api_key(&self) -> String {
        crate::settings::load().llm.and_then(|l| l.api_key).unwrap_or_default()
    }

    pub fn llm_model(&self) -> String {
        crate::settings::load()
            .llm
            .and_then(|l| l.model)
            .unwrap_or_else(|| "gemini-3.1-flash-lite".to_string())
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.data_dir.join("projects")
    }

    /// Stable per-project slug derived from the absolute project path.
    pub fn project_dir(&self, root: &Path) -> PathBuf {
        let slug = crate::hash::md5_hex(root.to_string_lossy().as_bytes())[..16].to_string();
        self.projects_dir().join(slug)
    }

    pub fn project_db(&self, root: &Path) -> PathBuf {
        self.project_dir(root).join("index.db")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(self.projects_dir())
            .with_context(|| format!("create {}", self.projects_dir().display()))?;
        Ok(())
    }

    /// Active project pointer, kept for parity with `active_project.txt`.
    pub fn set_active(&self, root: &Path) -> Result<()> {
        self.ensure_dirs()?;
        std::fs::write(self.data_dir.join("active_project.txt"), root.to_string_lossy().as_bytes())?;
        Ok(())
    }

    pub fn active_project(&self) -> Option<PathBuf> {
        std::fs::read_to_string(self.data_dir.join("active_project.txt"))
            .ok()
            .map(|s| PathBuf::from(s.trim()))
    }
}

use std::path::Path;

/// Registered project metadata (the Rust port of the Python `ProjectManager`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub indexed_at: Option<u64>,
    #[serde(default)]
    pub file_count: Option<u64>,
}

pub fn registry_path(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("projects.json")
}

pub fn load_registry(cfg: &Config) -> Vec<ProjectMeta> {
    std::fs::read_to_string(registry_path(cfg))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_registry(cfg: &Config, projects: &[ProjectMeta]) -> Result<()> {
    cfg.ensure_dirs()?;
    std::fs::write(
        registry_path(cfg),
        serde_json::to_vec_pretty(projects).context("serialize registry")?,
    )?;
    Ok(())
}

pub fn upsert_project(cfg: &Config, meta: ProjectMeta) -> Result<()> {
    let mut projects = load_registry(cfg);
    projects.retain(|p| p.path != meta.path);
    projects.push(meta);
    save_registry(cfg, &projects)
}