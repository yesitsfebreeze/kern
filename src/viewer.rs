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

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::base::graph::GraphGnn;
use crate::base::locks::read_recovered;
use crate::base::search::{find_entity, find_reason, search_all_unlocked, search_reasons_all_unlocked};
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
/// Upper bound on a single search request's `k`, so an over-large request can't
/// drive the HNSW `ef` budget into a multi-second scan while holding the read lock.
const MAX_SEARCH_K: usize = 200;

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
pub async fn run(graph: Graph, llm: crate::llm::Client, agg_addr: &str) -> std::io::Result<()> {
	// 1. Local graph server on an ephemeral loopback port (this daemon's own data).
	let local = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let local_addr = local.local_addr()?.to_string();
	let local_app = Router::new()
		.route("/graph", get(graph_json))
		.route("/search", post(peer_search))
		.with_state(graph);
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
				let hub = HubState { client: client.clone(), llm: llm.clone() };
				let app = Router::new()
					.route("/", get(index))
					.route("/graph", get(aggregate))
					.route("/search", get(hub_search))
					.with_state(hub);
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
async fn aggregate(State(st): State<HubState>) -> Json<Value> {
	let client = &st.client;
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

/// Tag one peer's search payload (`{hits, reasons}`) and append every hit to
/// `out`, prefixing `id`/`kern` so they match the namespaced ids `/graph`
/// already shipped to the browser. Both arrays are pooled into one list.
fn merge_search_hits(tag: &str, v: &Value, out: &mut Vec<Value>) {
	let pre = |id: &Value| -> Value {
		id.as_str().map(|s| Value::String(format!("{tag}{s}"))).unwrap_or(Value::Null)
	};
	for arr in ["hits", "reasons"] {
		for h in v.get(arr).and_then(Value::as_array).into_iter().flatten() {
			let mut h = h.clone();
			if let Some(o) = h.as_object_mut() {
				if let Some(id) = o.get("id") { let p = pre(id); o.insert("id".into(), p); }
				if let Some(k) = o.get("kern") { let p = pre(k); o.insert("kern".into(), p); }
			}
			out.push(h);
		}
	}
}

/// Merge every peer's tagged payload, sort by `score` descending, truncate to k.
fn rank_peers(peers: &[(String, Value)], k: usize) -> Vec<Value> {
	let mut out = Vec::new();
	for (tag, v) in peers {
		merge_search_hits(tag, v, &mut out);
	}
	out.sort_by(|a, b| {
		let sa = a.get("score").and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY);
		let sb = b.get("score").and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY);
		sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
	});
	out.truncate(k);
	out
}

fn default_k() -> usize { 10 }

#[derive(Clone)]
struct HubState {
	client: reqwest::Client,
	llm: crate::llm::Client,
}

#[derive(serde::Deserialize)]
struct SearchQuery {
	q: String,
	#[serde(default = "default_k")]
	k: usize,
}

/// Hub endpoint: embed the query ONCE, fan the vector out to every live peer's
/// `POST /search`, namespace + merge + rank. Embed failure (e.g. ollama down)
/// returns 503 so the browser can fall back to its in-page substring list.
async fn hub_search(State(st): State<HubState>, Query(p): Query<SearchQuery>) -> Response {
	let q = p.q.trim();
	if q.is_empty() {
		return Json(json!({ "results": [] })).into_response();
	}
	let vec = match st.llm.embed(q).await {
		Ok(v) => v,
		Err(e) => {
			tracing::warn!(target: "kern.viewer", error = %e, "search embed failed");
			return (StatusCode::SERVICE_UNAVAILABLE, "embed unavailable").into_response();
		}
	};

	let peers = live_peers();
	let body = json!({ "vec": vec, "k": p.k });
	let mut tagged = Vec::new();
	for addr in &peers {
		let url = format!("http://{addr}/search");
		let resp = match st.client.post(&url).json(&body).send().await {
			Ok(r) => r,
			Err(_) => continue, // unreachable peer (race with shutdown) — skip
		};
		if let Ok(v) = resp.json::<Value>().await {
			tagged.push((format!("{addr}|"), v));
		}
	}

	Json(json!({ "results": rank_peers(&tagged, p.k) })).into_response()
}

#[derive(serde::Deserialize)]
struct SearchBody {
	vec: Vec<f64>,
	#[serde(default = "default_k")]
	k: usize,
}

/// Peer endpoint: rank this daemon's graph against a *supplied* query vector.
/// No embedding happens here — the hub already embedded once and passes the
/// vector down, so N daemons cost one embed call total.
async fn peer_search(State(g): State<Graph>, Json(body): Json<SearchBody>) -> Json<Value> {
	let g = read_recovered(&g);
	let k = body.k.min(MAX_SEARCH_K);

	let mut hits = Vec::new();
	for h in search_all_unlocked(&g, &body.vec, k) {
		if let Some((e, kern)) = find_entity(&g, &h.entity_id) {
			hits.push(json!({
				"id": e.id,
				"label": truncate(&e.text(), 60),
				"kind": format!("{:?}", e.kind),
				"kern": kern,
				"heat": e.heat,
				"conf": e.conf_mean(),
				"score": h.score,
			}));
		}
	}

	let mut reasons = Vec::new();
	for h in search_reasons_all_unlocked(&g, &body.vec, k) {
		if let Some((r, kern)) = find_reason(&g, &h.reason_id) {
			// id is the edge's target entity so a click anchors a real node,
			// matching today's substring behavior (results used l.target).
			reasons.push(json!({
				"id": r.to,
				"label": truncate(&r.text, 80),
				"kind": format!("{:?}", r.kind),
				"kern": kern,
				"score": h.score,
			}));
		}
	}

	Json(json!({ "hits": hits, "reasons": reasons }))
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

	#[test]
	fn rank_peers_namespaces_pools_sorts_and_truncates() {
		// Two peers. Each returns entity hits + reason hits with scores.
		let peer_a = json!({
			"hits":    [{ "id": "e1", "kern": "k1", "label": "a", "score": 0.40 }],
			"reasons": [{ "id": "e9", "kern": "k1", "label": "ra", "score": 0.95 }],
		});
		let peer_b = json!({
			"hits":    [{ "id": "e2", "kern": "k2", "label": "b", "score": 0.70 }],
			"reasons": [],
		});
		let tagged = vec![
			("A|".to_string(), peer_a),
			("B|".to_string(), peer_b),
		];
		let out = rank_peers(&tagged, 2);

		// Truncated to k=2, sorted by score desc across BOTH peers and BOTH arrays.
		assert_eq!(out.len(), 2);
		assert_eq!(out[0]["score"], 0.95);
		assert_eq!(out[1]["score"], 0.70);
		// ids + kern are namespaced by peer tag so they match what /graph shipped.
		assert_eq!(out[0]["id"], "A|e9");
		assert_eq!(out[0]["kern"], "A|k1");
		assert_eq!(out[1]["id"], "B|e2");
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
