use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnswerConfig {
	pub url: String,
	pub model: String,
	pub key: String,
}

// Aliases the reason default: one runner serves both LLM legs.
pub const DEFAULT_ANSWER_MODEL: &str = super::reason::DEFAULT_REASON_MODEL;

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
		let c = AnswerConfig::default();
		assert!(c.url.is_empty(), "url empty -> falls back to reason/embed");
		assert!(c.key.is_empty(), "key empty -> inherits reason/embed key");
		assert_eq!(
			c.model, DEFAULT_ANSWER_MODEL,
			"model is the shared default const"
		);
	}
}
