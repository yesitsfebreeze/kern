use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
	pub addr: String,
	pub core_addr: String,
	// `gossip` is UDP: its port lives in a separate namespace from the TCP binds.
	pub gossip: String,
	pub mcp_sse: String,
}

impl Default for ServeConfig {
	fn default() -> Self {
		Self {
			addr: ":8080".into(),
			core_addr: ":2666".into(),
			gossip: ":7946".into(),
			mcp_sse: ":3000".into(),
		}
	}
}

impl ServeConfig {
	pub fn validate(&self) -> Result<(), String> {
		let mut seen: HashMap<u16, &'static str> = HashMap::new();
		for (name, addr) in [
			("addr", &self.addr),
			("core_addr", &self.core_addr),
			("mcp_sse", &self.mcp_sse),
		] {
			if addr.is_empty() {
				continue;
			}
			let Some(port) = addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) else {
				continue;
			};
			if port == 0 {
				continue;
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
			addr: String::new(),
			core_addr: ":0".into(),
			mcp_sse: ":0".into(),
			..Default::default()
		};
		assert!(cfg.validate().is_ok());
	}
}
