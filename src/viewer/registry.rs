//! Peer discovery via a shared registry directory.
//!
//! Every daemon writes `<temp>/kern-viewers/<pid>.json` and refreshes its
//! timestamp on a timer. The hub reads the directory to find live peers; stale
//! files (no heartbeat within [`STALE`]) are swept on read.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// Heartbeat cadence and the staleness window for treating a registry entry as
/// dead. A peer is live if its file was refreshed within `STALE` *and* its
/// `/graph` answers; otherwise the aggregator skips it.
const HEARTBEAT: Duration = Duration::from_secs(5);
const STALE: Duration = Duration::from_secs(20);

fn now_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
}

fn registry_dir() -> PathBuf {
	std::env::temp_dir().join("kern-viewers")
}

fn registry_file() -> PathBuf {
	registry_dir().join(format!("{}.json", std::process::id()))
}

/// Write `<temp>/kern-viewers/<pid>.json` once, then refresh its timestamp on a
/// timer. Best-effort: registry failures degrade to "this daemon is invisible
/// to the hub", never crash the daemon.
pub(super) fn spawn_registry(local_addr: String) {
	tokio::spawn(async move {
		let _ = std::fs::create_dir_all(registry_dir());
		let file = registry_file();
		loop {
			let body = json!({ "graph": local_addr, "ts": now_secs() }).to_string();
			let _ = std::fs::write(&file, &body);
			tokio::time::sleep(HEARTBEAT).await;
		}
	});
}

/// Read the registry directory and return the loopback `/graph` addresses of
/// every peer heartbeated within `STALE`. Stale files are swept.
pub(super) fn live_peers() -> Vec<String> {
	let dir = registry_dir();
	let entries = match std::fs::read_dir(&dir) {
		Ok(e) => e,
		Err(_) => return Vec::new(),
	};
	let now = now_secs();
	let mut peers = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		let Ok(text) = std::fs::read_to_string(&path) else {
			continue;
		};
		let Ok(v) = serde_json::from_str::<Value>(&text) else {
			continue;
		};
		let ts = v.get("ts").and_then(Value::as_u64).unwrap_or(0);
		if now.saturating_sub(ts) > STALE.as_secs() {
			let _ = std::fs::remove_file(&path); // sweep dead daemons
			continue;
		}
		if let Some(addr) = v.get("graph").and_then(Value::as_str) {
			peers.push(addr.to_string());
		}
	}
	peers
}
