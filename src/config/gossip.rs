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
	pub ingest_clip_enabled: bool,
	pub ingest_clip_max_per_window: u64,
	pub ingest_clip_window_secs: u64,
	pub trimmed_mean_enabled: bool,
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
