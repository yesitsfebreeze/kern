//! The per-daemon local server: serves THIS daemon's own graph, edits entities
//! and reasons, and executes kern MCP tools locally. The hub fans out to these
//! endpoints across every live peer.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use super::{Graph, MAX_SEARCH_K};
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::util::truncate;
use crate::config::RetrievalConfig;
use crate::tick::queue::{task, TaskKind};
use trnsprt::McpServer as _;

#[derive(Clone)]
pub(super) struct LocalState {
	pub(super) graph: Graph,
	pub(super) retrieval: RetrievalConfig,
	pub(super) queue: std::sync::Arc<crate::tick::queue::Queue>,
	pub(super) mcp: Arc<crate::mcp::Server>,
}

fn default_k() -> usize {
	10
}

#[derive(serde::Deserialize)]
pub(super) struct AskRetrieveBody {
	vec: Vec<f64>,
	question: String,
	#[serde(default = "default_k")]
	k: usize,
}

/// Peer endpoint for the oracle: retrieve (no generation) over THIS daemon's
/// graph and return scored source thoughts + a pre-formatted provenance string.
/// The hub merges these across daemons and does the single generation.
pub(super) async fn ask_retrieve(
	State(st): State<LocalState>,
	Json(body): Json<AskRetrieveBody>,
) -> Json<Value> {
	use crate::retrieval::answer;
	use crate::retrieval::seed::Mode;
	let k = body.k.min(MAX_SEARCH_K);
	let g = read_recovered(&st.graph);
	let result = answer::query(
		&g,
		&st.retrieval,
		&body.vec,
		&body.question,
		Mode::Hybrid,
		None,
		None,
		None::<crate::retrieval::score::QueryOptions>,
	);
	let sources: Vec<Value> = result
		.entities
		.iter()
		.take(k)
		.map(|se| {
			json!({
				"id": se.entity.id,
				"label": truncate(&se.entity.text(), 80),
				"text": truncate(&se.entity.text(), 300),
				"kind": format!("{:?}", se.entity.kind),
				"kern": g.kern_of_entity(&se.entity.id).map(str::to_owned).unwrap_or_default(),
				"heat": se.entity.heat,
				"conf": se.entity.conf_mean(),
				"score": se.score,
			})
		})
		.collect();
	let chain_text = answer::format_chains(&g, &result.path_chains);
	let mut reasons: Vec<Value> = Vec::new();
	let mut seen = std::collections::HashSet::new();
	for chain in &result.path_chains {
		for (j, node_id) in chain.nodes.iter().enumerate() {
			if j % 2 == 0 {
				continue;
			} // even = entity, odd = reason
			if !seen.insert(node_id.clone()) {
				continue;
			}
			if let Some((r, _)) = crate::base::search::find_reason(&g, node_id) {
				reasons.push(json!({
					"id": r.id,
					"from": r.from,
					"to": r.to,
					"text": if !r.text.is_empty() {
						truncate(&r.text, 160).to_string()
					} else {
						r.kind.fallback_label().unwrap_or("").to_string()
					},
					"kind": format!("{:?}", r.kind),
				}));
			}
		}
	}
	Json(json!({ "sources": sources, "chain_text": chain_text, "reasons": reasons }))
}

#[derive(serde::Deserialize)]
pub(super) struct ToolBody {
	name: String,
	#[serde(default)]
	args: Value,
}

/// Peer endpoint: execute a kern MCP tool locally and return the result.
pub(super) async fn local_tool(
	State(st): State<LocalState>,
	Json(body): Json<ToolBody>,
) -> Json<Value> {
	match st.mcp.call_tool(&body.name, &body.args) {
		Ok(r) => {
			let text = r
				.content
				.iter()
				.filter_map(|c: &Value| c.get("text").and_then(Value::as_str))
				.collect::<Vec<_>>()
				.join("\n");
			Json(json!({ "ok": !r.is_error, "result": text }))
		}
		Err(e) => Json(json!({ "ok": false, "error": format!("{e:?}") })),
	}
}

#[derive(serde::Deserialize)]
pub(super) struct EditBody {
	id: String,
	text: String,
	#[serde(default)]
	kind: String,
}

/// Peer endpoint: edit an entity or reason by id, mark dirty, enqueue reembed + persist.
pub(super) async fn edit(State(st): State<LocalState>, Json(body): Json<EditBody>) -> Json<Value> {
	let is_reason = body.kind == "reason";
	let kern_id = {
		let g = read_recovered(&st.graph);
		if is_reason {
			g.kern_of_reason(&body.id).map(|s| s.to_string())
		} else {
			g.kern_of_entity(&body.id).map(|s| s.to_string())
		}
	};
	let Some(kern_id) = kern_id else {
		return Json(json!({ "ok": false, "error": "not found" }));
	};
	{
		let mut g = write_recovered(&st.graph);
		if let Some(k) = g.get_mut(&kern_id) {
			if is_reason {
				if let Some(r) = k.reasons.get_mut(&body.id) {
					r.set_text(body.text.clone());
				}
			} else if let Some(e) = k.entities.get_mut(&body.id) {
				e.set_text(body.text.clone());
			}
		}
	}
	st.queue.enqueue(task(TaskKind::Reembed, &kern_id));
	st.queue.enqueue(task(TaskKind::Persist, &kern_id));
	Json(json!({ "ok": true }))
}

/// Snapshot the live graph as `{nodes, links, kerns}`. Nodes are entities
/// (id, truncated text, kind, kern, heat, confidence); links are reason edges.
/// Edges whose endpoints are not both present (e.g. into an unloaded kern) are
/// dropped so the client never sees a dangling link.
pub(super) async fn graph_json(State(st): State<LocalState>) -> Json<serde_json::Value> {
	let g = st.graph;
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
					"id": r.id,
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
				"label": if k.anchor_text.trim().is_empty() { "(unnamed)".to_string() } else { truncate(&k.anchor_text, 60) },
				"named": !k.anchor_text.trim().is_empty(),
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
