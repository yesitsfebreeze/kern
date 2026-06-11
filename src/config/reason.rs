use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasonConfig {
	/// Reasoning LLM endpoint (Ollama-native `/api/*`). When empty, the
	/// `Config::reason_url` accessor falls back to the embed endpoint.
	pub url: String,
	/// Model tag served at `url`, used for distillation / naming / edge-proposal.
	pub model: String,
	/// API key sent as a Bearer token for an authenticated endpoint. Empty means
	/// unauthenticated — the default for a local Ollama. Mirrors `EmbedConfig.key`.
	pub key: String,
}

/// Default reasoning endpoint — the same local Ollama that serves embeddings
/// ([`crate::config::DEFAULT_EMBED_URL`]). Empty-by-default broke the distill
/// and answer paths for any kern without an explicit `[reason] url`.
pub const DEFAULT_REASON_URL: &str = "http://localhost:11434";

/// Default reasoning model: `qwen2.5:7b` — small, fast, reliable structured-output
/// model, which is what distillation / naming / edge-proposal need. Exposed so
/// callers can reference the baseline without constructing a full `ReasonConfig`.
pub const DEFAULT_REASON_MODEL: &str = "qwen2.5:7b";

impl Default for ReasonConfig {
	fn default() -> Self {
		Self {
			url: DEFAULT_REASON_URL.into(),
			model: DEFAULT_REASON_MODEL.into(),
			key: String::new(),
		}
	}
}
