use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasonConfig {
	pub url: String,
	pub model: String,
	pub key: String,
}

const DEFAULT_REASON_URL: &str = "http://localhost:11434";

pub const DEFAULT_REASON_MODEL: &str = "granite4:3b";

impl Default for ReasonConfig {
	fn default() -> Self {
		Self {
			url: DEFAULT_REASON_URL.into(),
			model: DEFAULT_REASON_MODEL.into(),
			key: String::new(),
		}
	}
}
