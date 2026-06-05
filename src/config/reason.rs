use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasonConfig {
	pub url: String,
	pub model: String,
	pub key: String,
}

impl Default for ReasonConfig {
	fn default() -> Self {
		Self {
			url: String::new(),
			model: "qwen2.5".into(),
			key: String::new(),
		}
	}
}
