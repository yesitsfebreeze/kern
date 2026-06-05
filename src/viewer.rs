//! Live graph data API + zero-config local aggregator.
//!
//! Each kern daemon is per-cwd. To let one Vite app show *every* running kern
//! on the machine with no configuration, the viewer has two layers:
//!
//! 1. **Local server** — every daemon binds an ephemeral loopback port and
//!    serves its own graph at `GET /graph`. It writes that address into a
//!    shared registry directory (`<temp>/kern-viewers/<pid>.json`) and
//!    heartbeats it. A browser can't read UDP broadcasts, so the registry is a
//!    file the aggregator (a process, not the browser) reads.
//! 2. **Aggregator** — every daemon races to bind the well-known address
//!    `cfg.serve.viewer` (default 127.0.0.1:7700). Exactly one wins and becomes
//!    the hub; the rest retry periodically so the hub fails over if it dies.
//!    The hub serves `GET /graph` by fanning out to every live peer in the
//!    registry, namespacing their ids, and merging into one `{nodes,links,kerns}`.
//!
//! The browser always fetches `127.0.0.1:7700/graph` and gets the union — zero
//! config whether one daemon runs or ten.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::base::graph::GraphGnn;
use crate::base::locks::read_recovered;
use crate::base::util::truncate;

type Graph = Arc<RwLock<GraphGnn>>;

/// Heartbeat cadence and the staleness window for treating a registry entry as
/// dead. A peer is live if its file was refreshed within `STALE` *and* its
/// `/graph` answers; otherwise the aggregator skips it.
const HEARTBEAT: Duration = Duration::from_secs(5);
const STALE: Duration = Duration::from_secs(20);
/// Per-peer fan-out timeout: a wedged daemon must not stall the whole view.
const FANOUT_TIMEOUT: Duration = Duration::from_secs(3);
/// How often a non-hub daemon retries binding the aggregator address.
const FAILOVER_RETRY: Duration = Duration::from_secs(4);

fn now_secs() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn registry_dir() -> PathBuf {
	std::env::temp_dir().join("kern-viewers")
}

fn registry_file() -> PathBuf {
	registry_dir().join(format!("{}.json", std::process::id()))
}

/// Run the viewer: start this daemon's local graph server, register it, and
/// contend for the aggregator role. Never returns under normal operation.
pub async fn run(graph: Graph, agg_addr: &str) -> std::io::Result<()> {
	// 1. Local graph server on an ephemeral loopback port (this daemon's own data).
	let local = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let local_addr = local.local_addr()?.to_string();
	let local_app = Router::new().route("/graph", get(graph_json)).with_state(graph);
	tokio::spawn(async move {
		if let Err(e) = axum::serve(local, local_app).await {
			tracing::warn!(target: "kern.viewer", error = %e, "local graph server exited");
		}
	});
	tracing::info!(target: "kern.viewer", addr = %local_addr, "local graph server listening");

	// 2. Register self + heartbeat so the hub can discover this daemon.
	spawn_registry(local_addr.clone());

	// 3. Contend for the aggregator address; retry so the hub can fail over.
	let client = reqwest::Client::builder()
		.timeout(FANOUT_TIMEOUT)
		.build()
		.unwrap_or_default();
	let agg_addr = agg_addr.to_string();
	loop {
		match tokio::net::TcpListener::bind(&agg_addr).await {
			Ok(listener) => {
				tracing::info!(target: "kern.viewer", addr = %agg_addr, "aggregator hub listening");
				let app = Router::new()
					.route("/", get(index))
					.route("/graph", get(aggregate))
					.with_state(client.clone());
				if let Err(e) = axum::serve(listener, app).await {
					tracing::warn!(target: "kern.viewer", error = %e, "aggregator hub exited; will retry");
				}
			}
			// Another daemon holds the hub. Wait, then retry to take over if it dies.
			Err(_) => tokio::time::sleep(FAILOVER_RETRY).await,
		}
	}
}

/// Write `<temp>/kern-viewers/<pid>.json` once, then refresh its timestamp on a
/// timer. Best-effort: registry failures degrade to "this daemon is invisible
/// to the hub", never crash the daemon.
fn spawn_registry(local_addr: String) {
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
fn live_peers() -> Vec<String> {
	let dir = registry_dir();
	let entries = match std::fs::read_dir(&dir) {
		Ok(e) => e,
		Err(_) => return Vec::new(),
	};
	let now = now_secs();
	let mut peers = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		let Ok(text) = std::fs::read_to_string(&path) else { continue };
		let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
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

async fn index() -> &'static str {
	"kern viewer aggregator. GET /graph for the merged graph across all running daemons."
}

/// Hub endpoint: fan out to every live peer, namespace ids per peer to avoid
/// cross-project collisions, and merge into one `{nodes,links,kerns}`.
async fn aggregate(State(client): State<reqwest::Client>) -> Json<Value> {
	let peers = live_peers();
	let mut nodes = Vec::new();
	let mut links = Vec::new();
	let mut kerns = Vec::new();

	for addr in &peers {
		let url = format!("http://{addr}/graph");
		let resp = match client.get(&url).send().await {
			Ok(r) => r,
			Err(_) => continue, // unreachable peer (race with shutdown) — skip
		};
		let Ok(v) = resp.json::<Value>().await else { continue };
		// Namespace by peer address so identical ids in different daemons (e.g.
		// the same Fact text hashing alike across projects) never merge or
		// shadow. Links stay valid because endpoints share the peer's tag.
		let tag = format!("{addr}|");
		merge_peer(&tag, &v, &mut nodes, &mut links, &mut kerns);
	}

	Json(json!({
		"nodes": nodes,
		"links": links,
		"kerns": kerns,
		"kern_count": kerns.len(),
		"daemons": peers.len(),
	}))
}

/// Re-key one peer's payload under `tag` and append to the merged arrays.
fn merge_peer(tag: &str, v: &Value, nodes: &mut Vec<Value>, links: &mut Vec<Value>, kerns: &mut Vec<Value>) {
	let pre = |id: &Value| -> Value {
		id.as_str().map(|s| Value::String(format!("{tag}{s}"))).unwrap_or(Value::Null)
	};
	for n in v.get("nodes").and_then(Value::as_array).into_iter().flatten() {
		let mut n = n.clone();
		if let Some(o) = n.as_object_mut() {
			if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
			if let Some(k) = o.get("kern") { let p = pre(k); o.insert("kern".into(), p); }
		}
		nodes.push(n);
	}
	for l in v.get("links").and_then(Value::as_array).into_iter().flatten() {
		let mut l = l.clone();
		if let Some(o) = l.as_object_mut() {
			if let Some(s) = o.get("source") { let p = pre(s); o.insert("source".into(), p); }
			if let Some(t) = o.get("target") { let p = pre(t); o.insert("target".into(), p); }
		}
		links.push(l);
	}
	for k in v.get("kerns").and_then(Value::as_array).into_iter().flatten() {
		let mut k = k.clone();
		if let Some(o) = k.as_object_mut() {
			if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
			match o.get("parent") {
				Some(p) if p.is_string() => { let np = pre(p); o.insert("parent".into(), np); }
				_ => {}
			}
			if let Some(ch) = o.get("children").and_then(Value::as_array) {
				let mapped: Vec<Value> = ch.iter().map(&pre).collect();
				o.insert("children".into(), Value::Array(mapped));
			}
		}
		kerns.push(k);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn merge_peer_namespaces_ids_and_keeps_links_valid() {
		let payload = json!({
			"nodes": [{ "id": "e1", "kern": "k1", "label": "x" }],
			"links": [{ "source": "e1", "target": "e2", "kind": "Supports" }],
			"kerns": [
				{ "id": "k0", "parent": null, "children": ["k1"] },
				{ "id": "k1", "parent": "k0", "children": [] },
			],
		});
		let (mut n, mut l, mut k) = (Vec::new(), Vec::new(), Vec::new());
		merge_peer("127.0.0.1:7701|", &payload, &mut n, &mut l, &mut k);

		assert_eq!(n[0]["id"], "127.0.0.1:7701|e1");
		assert_eq!(n[0]["kern"], "127.0.0.1:7701|k1");
		// Endpoints carry the same tag, so the edge still resolves post-merge.
		assert_eq!(l[0]["source"], "127.0.0.1:7701|e1");
		assert_eq!(l[0]["target"], "127.0.0.1:7701|e2");
		// Root stays parentless; child parent/children references are re-keyed.
		assert!(k[0]["parent"].is_null());
		assert_eq!(k[0]["children"][0], "127.0.0.1:7701|k1");
		assert_eq!(k[1]["parent"], "127.0.0.1:7701|k0");
	}

	#[test]
	fn merge_peer_tolerates_missing_arrays() {
		let (mut n, mut l, mut k) = (Vec::new(), Vec::new(), Vec::new());
		merge_peer("t|", &json!({}), &mut n, &mut l, &mut k);
		assert!(n.is_empty() && l.is_empty() && k.is_empty());
	}
}

/// Snapshot the live graph as `{nodes, links, kerns}`. Nodes are entities
/// (id, truncated text, kind, kern, heat, confidence); links are reason edges.
/// Edges whose endpoints are not both present (e.g. into an unloaded kern) are
/// dropped so the client never sees a dangling link.
async fn graph_json(State(g): State<Graph>) -> Json<serde_json::Value> {
	let g = read_recovered(&g);
	let kerns = g.all();

	let mut node_ids: HashSet<String> = HashSet::new();
	let mut nodes = Vec::new();
	for kern in &kerns {
		for e in kern.entities.values() {
			node_ids.insert(e.id.clone());
			nodes.push(json!({
				"id": e.id,
				"label": truncate(&e.text(), 60),
				"kind": format!("{:?}", e.kind),
				"kern": kern.id,
				"heat": e.heat,
				"conf": e.conf_mean(),
			}));
		}
	}

	let mut links = Vec::new();
	for kern in &kerns {
		for r in kern.reasons.values() {
			if node_ids.contains(&r.from) && node_ids.contains(&r.to) {
				links.push(json!({
					"source": r.from,
					"target": r.to,
					"kind": format!("{:?}", r.kind),
					"text": truncate(&r.text, 80),
					"score": r.score,
				}));
			}
		}
	}

	// Sphere structure: the recursive kern tree (purpose, radii, parent/children,
	// member count). The viewer renders each kern as a sphere you can step into.
	let kern_meta: Vec<_> = kerns
		.iter()
		.map(|k| {
			json!({
				"id": k.id,
				"label": if k.purpose_text.trim().is_empty() { "(unnamed)".to_string() } else { truncate(&k.purpose_text, 60) },
				"named": !k.purpose_text.trim().is_empty(),
				"parent": k.parent,
				"children": k.children,
				"inner_radius": k.inner_radius,
				"outer_radius": k.outer_radius,
				"count": k.entities.len(),
			})
		})
		.collect();

	Json(json!({
		"nodes": nodes,
		"links": links,
		"kerns": kern_meta,
		"kern_count": kerns.len(),
	}))
}
