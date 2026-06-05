use serde::{Deserialize, Serialize};

/// Default embedding endpoint (local Ollama). Single source of truth shared
/// by [`EmbedConfig::default`] and the CLI `--embed-url` clap default.
pub const DEFAULT_EMBED_URL: &str = "http://localhost:11434";
/// Default embedding model. Shared by [`EmbedConfig::default`] and the CLI
/// `--embed-model` clap default. qwen3-embedding:0.6b: small (~640 MB), fast,
/// and higher retrieval quality than nomic/mxbai (tops MTEB for its class).
/// NOTE: the embedding dimension is locked into the graph on first ingest —
/// changing this later requires `kern reembed`.
pub const DEFAULT_EMBED_MODEL: &str = "qwen3-embedding:0.6b";

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
