use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::util::expand_tilde;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where flat model directories are written. Defaults to ~/models.
    pub output_dir: String,
    /// HuggingFace endpoint (supports mirrors such as https://hf-mirror.com).
    pub endpoint: String,
    /// Access token for private and gated repositories.
    pub token: Option<String>,
    /// Parallel connections per file.
    pub connections: usize,
    /// Number of files downloaded concurrently.
    pub max_active: usize,
    /// Verify SHA-256 of every downloaded file when the hash is known.
    pub verify: bool,
    /// Proxy URL, e.g. http://host:8080 or socks5://host:1080.
    pub proxy: Option<String>,
    /// Launch the TUI when stdout is a terminal.
    pub tui: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_dir: "~/models".to_string(),
            endpoint: "https://huggingface.co".to_string(),
            token: None,
            connections: 8,
            max_active: 3,
            verify: false,
            proxy: None,
            tui: true,
        }
    }
}

impl Config {
    pub fn output_path(&self) -> PathBuf {
        expand_tilde(&self.output_dir)
    }

    pub fn endpoint(&self) -> String {
        self.endpoint.trim_end_matches('/').to_string()
    }
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| expand_tilde("~"))
        .join(".config")
        .join("hfd")
        .join("config.toml")
}

/// Load configuration from disk and apply environment overrides.
pub fn load() -> Config {
    let mut config = std::fs::read_to_string(config_path())
        .ok()
        .and_then(|text| toml::from_str::<Config>(&text).ok())
        .unwrap_or_default();

    if let Ok(token) = std::env::var("HF_TOKEN")
        && !token.is_empty()
    {
        config.token = Some(token);
    }
    if let Ok(endpoint) = std::env::var("HF_ENDPOINT")
        && !endpoint.is_empty()
    {
        config.endpoint = endpoint;
    }
    if let Ok(output) = std::env::var("HFD_OUTPUT_DIR")
        && !output.is_empty()
    {
        config.output_dir = output;
    }
    if let Ok(proxy) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY"))
        && !proxy.is_empty()
        && config.proxy.is_none()
    {
        config.proxy = Some(proxy);
    }
    config
}

pub fn save(config: &Config) -> Result<PathBuf> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = toml::to_string_pretty(config)?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}
