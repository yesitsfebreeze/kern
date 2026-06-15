use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
	/// HTTP RPC + MCP-over-HTTP bind address (the primary client-facing endpoint).
	pub addr: String,
	/// Internal core typed-RPC socket — the kern_rpc surface used by sub-agents
	/// and other in-host processes (not the public HTTP API).
	pub core_addr: String,
	/// Federation gossip bind address. **UDP** (discovery + state sync between
	/// peer daemons) — distinct from the TCP listeners above.
	pub gossip: String,
	/// MCP Server-Sent-Events streaming endpoint (push transport for MCP clients).
	pub mcp_sse: String,
	/// Live graph viewer bind address. Localhost-only by default; empty = off.
	pub viewer: String,
}

impl Default for ServeConfig {
	fn default() -> Self {
		Self {
			addr: ":8080".into(),
			core_addr: ":2666".into(),
			gossip: ":7946".into(),
			mcp_sse: ":3000".into(),
			viewer: "127.0.0.1:7700".into(),
		}
	}
}

impl ServeConfig {
	/// Catch an obviously-broken layout before bind: two TCP listeners sharing a
	/// port (which would make one fail at startup with a confusing OS error).
	/// `gossip` is excluded — it is UDP, so its port lives in a separate
	/// namespace and never clashes with the TCP endpoints. Empty (disabled),
	/// port-0 (ephemeral), and unparseable addresses are skipped. Returns the
	/// offending port + field names on conflict.
	pub fn validate(&self) -> Result<(), String> {
		let mut seen: HashMap<u16, &'static str> = HashMap::new();
		for (name, addr) in [
			("addr", &self.addr),
			("core_addr", &self.core_addr),
			("mcp_sse", &self.mcp_sse),
			("viewer", &self.viewer),
		] {
			if addr.is_empty() {
				continue;
			}
			let Some(port) = addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) else {
				continue; // unparseable host:port -> leave it for the bind to surface
			};
			if port == 0 {
				continue; // ephemeral port, never a real clash
			}
			if let Some(prev) = seen.insert(port, name) {
				return Err(format!(
					"duplicate TCP bind port {port} on `{prev}` and `{name}`"
				));
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_config_has_no_port_clash() {
		assert!(ServeConfig::default().validate().is_ok());
	}

	#[test]
	fn duplicate_tcp_port_is_rejected() {
		let cfg = ServeConfig {
			addr: ":9000".into(),
			mcp_sse: ":9000".into(),
			..Default::default()
		};
		let err = cfg.validate().unwrap_err();
		assert!(err.contains("9000"), "names the clashing port: {err}");
	}

	#[test]
	fn udp_gossip_sharing_a_tcp_port_is_allowed() {
		// gossip is UDP; reusing a TCP port number is not a real conflict.
		let cfg = ServeConfig {
			addr: ":8080".into(),
			gossip: ":8080".into(),
			..Default::default()
		};
		assert!(cfg.validate().is_ok());
	}

	#[test]
	fn empty_and_ephemeral_addrs_are_skipped() {
		let cfg = ServeConfig {
			addr: String::new(),    // disabled
			core_addr: ":0".into(), // ephemeral
			mcp_sse: ":0".into(),   // ephemeral — two :0 must NOT collide
			viewer: String::new(),
			..Default::default()
		};
		assert!(cfg.validate().is_ok());
	}
}
