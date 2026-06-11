use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use tokio::net::UdpSocket;

use crate::base::constants::{GOSSIP_DISCOVERY_INTERVAL, GOSSIP_DISCOVERY_MULTICAST};

use super::node::Node;

const ANNOUNCE_PREFIX: &str = "kern:";

/// Periodically announce this node on the discovery multicast group so same-LAN
/// peers can find it with zero configuration. Every `GOSSIP_DISCOVERY_INTERVAL`
/// it sends `kern:<network_id>:<tcp_addr>` to `GOSSIP_DISCOVERY_MULTICAST:port`
/// from an ephemeral UDP socket. Counterpart to [`start_listen`]; the spawned
/// task runs until the node's stop signal fires.
pub fn start_broadcast(node: &Arc<Node>, port: u16) {
	let node = node.clone();
	let addr: SocketAddr = match format!("{GOSSIP_DISCOVERY_MULTICAST}:{port}").parse() {
		Ok(a) => a,
		Err(_) => return,
	};
	tokio::spawn(async move {
		let socket = match UdpSocket::bind("0.0.0.0:0").await {
			Ok(s) => s,
			Err(_) => return,
		};

		let payload = format!("{ANNOUNCE_PREFIX}{}:{}", node.network_id, node.addr());
		let payload_bytes = payload.as_bytes();

		let mut interval = tokio::time::interval(GOSSIP_DISCOVERY_INTERVAL);
		let mut stop = node.stop_rx.clone();
		loop {
			tokio::select! {
				_ = interval.tick() => {
					let _ = socket.send_to(payload_bytes, addr).await;
				}
				_ = stop.changed() => break,
			}
		}
	});
}

/// Listen for peer announcements on the discovery multicast group and add
/// matching peers (same network id, not ourselves). Counterpart to
/// `start_broadcast` — together they give zero-config LAN peering.
pub fn start_listen(node: &Arc<Node>, port: u16) {
	let node = node.clone();
	tokio::spawn(async move {
		let group: Ipv4Addr = match GOSSIP_DISCOVERY_MULTICAST.parse() {
			Ok(g) => g,
			Err(_) => return,
		};
		let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).await {
			Ok(s) => s,
			Err(_) => return,
		};
		let _ = socket.join_multicast_v4(group, Ipv4Addr::UNSPECIFIED);
		let mut stop = node.stop_rx.clone();
		let mut buf = [0u8; 512];
		loop {
			tokio::select! {
				_ = stop.changed() => break,
				// Awaited directly — no set_nonblocking + sleep-drain poll, and no
				// blocking recv pinning a worker thread off the async executor.
				r = socket.recv_from(&mut buf) => {
					if let Ok((n, _src)) = r {
						if let Ok(s) = std::str::from_utf8(&buf[..n]) {
							if let Some((nid, addr)) = parse_announce(s) {
								if nid == node.network_id && addr != node.addr() {
									node.add_peer(&addr);
								}
							}
						}
					}
				}
			}
		}
	});
}

pub fn parse_announce(s: &str) -> Option<(String, String)> {
	let s = s.strip_prefix(ANNOUNCE_PREFIX)?;
	if s.len() < 38 {
		return None;
	}
	let network_id = &s[..36];
	if s.as_bytes()[36] != b':' {
		return None;
	}
	let tcp_addr = &s[37..];
	Some((network_id.to_string(), tcp_addr.to_string()))
}

#[cfg(test)]
mod tests {
	use super::*;

	// A 36-char UUID-shaped network id, matching the fixed-width slice parse.
	const NID: &str = "123e4567-e89b-12d3-a456-426614174000";

	#[test]
	fn parse_announce_accepts_valid_payload() {
		let raw = format!("kern:{NID}:127.0.0.1:7400");
		let (nid, addr) = parse_announce(&raw).expect("valid announce parses");
		assert_eq!(nid, NID);
		assert_eq!(addr, "127.0.0.1:7400");
	}

	#[test]
	fn parse_announce_rejects_wrong_prefix() {
		let raw = format!("gossip:{NID}:127.0.0.1:7400");
		assert!(parse_announce(&raw).is_none(), "non-kern prefix is rejected");
	}

	#[test]
	fn parse_announce_rejects_too_short() {
		// Below the 38-byte minimum (36-char id + ':' + at least one addr byte).
		assert!(parse_announce("kern:short").is_none());
	}

	#[test]
	fn parse_announce_rejects_missing_separator_colon() {
		// Position 36 (just after the id) must be ':'; here it's 'X'.
		let raw = format!("kern:{NID}X127.0.0.1:7400");
		assert!(parse_announce(&raw).is_none(), "missing id/addr separator is rejected");
	}
}
