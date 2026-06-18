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
	/// UDP port for discovery announce/listen.
	pub discovery_port: u16,
	/// Seed peers (host:port) to dial on startup, in addition to discovery.
	pub peers: Vec<String>,
}

impl Default for GossipConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			addr: "0.0.0.0:7400".into(),
			discovery: true,
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
		assert!(c.peers.is_empty(), "no seed peers by default");
	}
}
