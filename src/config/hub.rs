use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct HubConfig {
	// `kern mcp` spawns a detached machine-level hub when none answers, same as
	// it already auto-spawns a project daemon. false = hub is opt-in via
	// `kern hub`; the legacy direct-connect path still works either way.
	pub auto_start: bool,
}

impl Default for HubConfig {
	fn default() -> Self {
		Self { auto_start: true }
	}
}
