use serde::{Deserialize, Serialize};

/// Model for the user-facing `/ask` oracle (streamed answer over MCP).
///
/// Split from [`crate::config::ReasonConfig`] on purpose: the two have OPPOSITE
/// optimization targets. Distillation/edge-proposal ([reason]) runs in the
/// background — latency is free, structured-output reliability matters, so it
/// wants a bigger model. The answer path is user-facing and only glues already-
/// retrieved graph nodes into prose, so it wants the smallest model that clears
/// the grounding floor. Keeping them one knob forced one to lose.
///
/// Empty `url`/`key` fall back to [reason] (which itself falls back to [embed]),
/// so a single local Ollama needs no extra wiring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnswerConfig {
	pub url: String,
	pub model: String,
	pub key: String,
}

/// Default answer model. qwen3.5:4b — dumb-fast glue over graph context, fits
/// 8 GB VRAM alongside the 0.6b embedder with KV headroom, same family as the
/// embedder so it shares Ollama's tokenizer cache.
pub const DEFAULT_ANSWER_MODEL: &str = "qwen3.5:4b";

impl Default for AnswerConfig {
	fn default() -> Self {
		Self {
			url: String::new(),
			model: DEFAULT_ANSWER_MODEL.into(),
			key: String::new(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_leaves_endpoint_empty_and_uses_the_answer_model() {
		// Empty url/key on purpose: the Config::answer_url/answer_key accessors then
		// fall back to [reason] (-> [embed]), so a single local Ollama needs no wiring.
		let c = AnswerConfig::default();
		assert!(c.url.is_empty(), "url empty -> falls back to reason/embed");
		assert!(c.key.is_empty(), "key empty -> inherits reason/embed key");
		assert_eq!(
			c.model, DEFAULT_ANSWER_MODEL,
			"model is the shared default const"
		);
	}
}
