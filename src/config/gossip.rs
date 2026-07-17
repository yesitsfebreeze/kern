use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GossipConfig {
	pub enabled: bool,
	pub addr: String,
	pub discovery: bool,
	pub network_id: Option<String>,
	pub discovery_port: u16,
	pub peers: Vec<String>,
}

impl GossipConfig {
	// A ':' in the id would corrupt the `kern:<id>:<addr>` announce wire format.
	pub fn effective_network_id(&self, generated: &str) -> String {
		match self.network_id.as_deref() {
			Some(id) if !id.is_empty() && !id.contains(':') => id.to_string(),
			Some(id) if !id.is_empty() => {
				tracing::warn!(
					target: "kern.gossip",
					network_id = %id,
					"[gossip] network_id must not contain ':'; falling back to the generated id"
				);
				generated.to_string()
			}
			_ => generated.to_string(),
		}
	}
}

impl Default for GossipConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			addr: "0.0.0.0:7400".into(),
			discovery: true,
			network_id: None,
			discovery_port: 7475,
			peers: Vec::new(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_is_disabled_with_expected_field_values() {
		let c = GossipConfig::default();
		assert!(!c.enabled, "gossip is disabled by default");
		assert_eq!(c.addr, "0.0.0.0:7400");
		assert!(
			c.discovery,
			"discovery defaults on (only matters once enabled)"
		);
		assert_eq!(c.discovery_port, 7475);
		assert!(
			c.network_id.is_none(),
			"no pooling id by default — each daemon keeps its unique generated id"
		);
		assert!(c.peers.is_empty(), "no seed peers by default");
	}

	#[test]
	fn effective_network_id_prefers_a_valid_configured_id() {
		let c = GossipConfig {
			network_id: Some("team-alpha".into()),
			..GossipConfig::default()
		};
		assert_eq!(c.effective_network_id("generated-uuid"), "team-alpha");
	}

	#[test]
	fn effective_network_id_falls_back_when_unset_empty_or_invalid() {
		let mut c = GossipConfig::default();
		assert_eq!(c.effective_network_id("gen"), "gen", "unset -> generated");
		c.network_id = Some(String::new());
		assert_eq!(c.effective_network_id("gen"), "gen", "empty -> generated");
		c.network_id = Some("has:colon".into());
		assert_eq!(
			c.effective_network_id("gen"),
			"gen",
			"':' would corrupt the announce wire format"
		);
	}
}
