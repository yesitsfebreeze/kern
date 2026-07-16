//! `[gossip]` config: opt-in federation that lets a kern daemon share knowledge
//! with same-network peers over TCP + LAN multicast discovery. OFF by default —
//! a lone daemon never opens a gossip socket or announces itself. Enable it per
//! project only when you want several daemons (sharing one `network_id`) to pool
//! memory.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GossipConfig {
	/// Master switch for federation. OFF by default: when false the daemon
	/// runs no gossip node, no listener, and no discovery (the historical
	/// behavior). Turn on per project to share knowledge with peers.
	pub enabled: bool,
	/// TCP bind address for the gossip listener. A fixed port lets peers
	/// dial in; `:0` picks an ephemeral port (discovery still advertises it).
	pub addr: String,
	/// LAN multicast peer discovery: advertise this node and auto-add peers
	/// announcing the same network id.
	pub discovery: bool,
	/// Discovery network id shared by the nodes to pool. When unset, the
	/// graph's generated per-daemon UUID is announced — unique per daemon, so
	/// independent daemons never auto-pair. Set the same value (any non-empty
	/// string without ':') on each daemon to pool them intentionally.
	pub network_id: Option<String>,
	/// UDP port for discovery announce/listen.
	pub discovery_port: u16,
	/// Seed peers (host:port) to dial on startup, in addition to discovery.
	pub peers: Vec<String>,
}

impl GossipConfig {
	/// The network id to announce on discovery: the configured `network_id`
	/// when valid, else the graph's `generated` one. A ':' would corrupt the
	/// `kern:<id>:<addr>` announce wire format, so such ids are rejected.
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
		// Federation is OFF by default — a fresh daemon must never open a socket or
		// announce itself without an explicit opt-in.
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
