use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use crate::base::constants::{GOSSIP_DISCOVERY_INTERVAL, GOSSIP_DISCOVERY_MULTICAST};

use super::node::Node;

const ANNOUNCE_PREFIX: &str = "kern:";

pub fn start_broadcast(node: &Arc<Node>, port: u16) {
	let node = node.clone();
	let addr = format!("{GOSSIP_DISCOVERY_MULTICAST}:{port}");
	tokio::spawn(async move {
		let socket = match UdpSocket::bind("0.0.0.0:0") {
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
					let _ = socket.send_to(payload_bytes, &addr);
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
		let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)) {
			Ok(s) => s,
			Err(_) => return,
		};
		let _ = socket.join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED);
		if socket.set_nonblocking(true).is_err() {
			return;
		}
		let mut stop = node.stop_rx.clone();
		let mut buf = [0u8; 512];
		loop {
			tokio::select! {
				_ = stop.changed() => break,
				_ = tokio::time::sleep(Duration::from_millis(500)) => {
					// Drain any pending datagrams (non-blocking).
					while let Ok((n, _src)) = socket.recv_from(&mut buf) {
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
