use serde::{Deserialize, Serialize};

/// Default embedding endpoint (local Ollama). Single source of truth shared
/// by [`EmbedConfig::default`] and the CLI `--embed-url` clap default.
pub const DEFAULT_EMBED_URL: &str = "http://localhost:11434";
/// Default embedding model. Shared by [`EmbedConfig::default`] and the CLI
/// `--embed-model` clap default.
pub const DEFAULT_EMBED_MODEL: &str = "bge-m3";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedConfig {
	pub url: String,
	pub model: String,
	pub key: String,
}

impl Default for EmbedConfig {
	fn default() -> Self {
		Self {
			url: DEFAULT_EMBED_URL.into(),
			model: DEFAULT_EMBED_MODEL.into(),
			key: String::new(),
		}
	}
}
