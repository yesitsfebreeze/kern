//! `[gossip]` config: opt-in federation that lets a kern daemon share knowledge
//! with same-network peers over TCP + LAN multicast discovery. OFF by default —
//! a lone daemon never opens a gossip socket or announces itself. Enable it per
//! project only when you want several daemons (sharing one `network_id`) to pool
//! memory; the `ingest_clip_*` and `trimmed_mean_*` knobs are the Sybil-flood and
//! outlier defenses for that shared, peer-writable surface.

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
	/// Rate-limit inbound gossip ingests per peer (Sybil-flood defense). When off,
	/// a peer can push unbounded entities into this node's phantom kerns.
	pub ingest_clip_enabled: bool,
	/// Max accepted gossip ingests from one peer per `ingest_clip_window_secs`
	/// before further ingests in that window are dropped.
	pub ingest_clip_max_per_window: u64,
	/// Length of the per-peer rate-limit window, in seconds.
	pub ingest_clip_window_secs: u64,
	/// Use a trimmed mean (drop the extreme tails) when fusing the same entity's
	/// scores reported by multiple peers, so one outlier peer can't skew the merge.
	pub trimmed_mean_enabled: bool,
	/// Fraction trimmed from EACH end before averaging peer-reported scores
	/// (e.g. 0.10 drops the top and bottom 10%).
	pub trimmed_mean_trim_pct: f64,
}

impl Default for GossipConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			addr: "0.0.0.0:7400".into(),
			discovery: true,
			discovery_port: 7475,
			peers: Vec::new(),
			ingest_clip_enabled: false,
			ingest_clip_max_per_window: 1000,
			ingest_clip_window_secs: 1,
			trimmed_mean_enabled: false,
			trimmed_mean_trim_pct: 0.10,
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
		assert!(c.discovery, "discovery defaults on (only matters once enabled)");
		assert_eq!(c.discovery_port, 7475);
		assert!(c.peers.is_empty(), "no seed peers by default");
		// Security-sensitive defenses default OFF but carry sane bounds.
		assert!(!c.ingest_clip_enabled);
		assert_eq!(c.ingest_clip_max_per_window, 1000);
		assert_eq!(c.ingest_clip_window_secs, 1);
		assert!(!c.trimmed_mean_enabled);
		assert!((c.trimmed_mean_trim_pct - 0.10).abs() < 1e-12);
	}
}
