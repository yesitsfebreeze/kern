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
	/// Embedding endpoint (Ollama-native `/api/embed`). Defaults to local Ollama.
	pub url: String,
	/// Embedding model tag. **Dimension-locked**: the vector dimension is fixed
	/// into the graph on first ingest, so switching models on an existing store
	/// requires `kern reembed` to re-vector every entity — otherwise the new
	/// dimension mismatches stored vectors and search silently misses. See
	/// [`DEFAULT_EMBED_MODEL`].
	pub model: String,
	/// API key sent as a Bearer token to a hosted/authenticated embedding endpoint.
	/// Empty for a local unauthenticated Ollama (the default).
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_uses_the_shared_constants() {
		// Guards against a silent regression where Default and the CLI/clap
		// defaults drift from the single-source-of-truth consts.
		let c = EmbedConfig::default();
		assert_eq!(c.url, DEFAULT_EMBED_URL);
		assert_eq!(c.model, DEFAULT_EMBED_MODEL);
		assert!(c.key.is_empty(), "no API key by default (local Ollama)");
	}
}
