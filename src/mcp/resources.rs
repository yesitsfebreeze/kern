use serde_json::value::RawValue;

use crate::base::locks::read_recovered;
use crate::base::search::{find_reason, find_entity};
use crate::base::util::truncate;

use super::{err_resp, ok, Response, Server, ERR_INVALID_REQ, ERR_NOT_FOUND};

pub fn resource_definitions() -> Vec<serde_json::Value> {
	vec![
		serde_json::json!({
			"uri": "kern://local/health",
			"name": "Graph health",
			"description": "Entity/edge counts, tick heat, unnamed count, purpose",
			"mimeType": "application/json",
		}),
		serde_json::json!({
			"uri": "kern://local/thoughts",
			"name": "Top thoughts",
			"description": "Top thoughts by global rank",
			"mimeType": "application/json",
		}),
		serde_json::json!({
			"uri": "kern://local/kerns",
			"name": "All Kerns",
			"description": "All loaded Kerns with purpose and stats",
			"mimeType": "application/json",
		}),
		serde_json::json!({
			"uri": "kern://local/descriptors",
			"name": "Descriptors",
			"description": "All registered data-type descriptors",
			"mimeType": "application/json",
		}),
	]
}

pub(crate) fn handle_resource_read(
	server: &Server,
	id: Option<Box<RawValue>>,
	params: Option<Box<RawValue>>,
) -> Response {
	#[derive(serde::Deserialize)]
	struct Params {
		uri: String,
	}

	let params: Params = match params
		.as_deref()
		.map(|r| serde_json::from_str(r.get()))
		.transpose()
	{
		Ok(Some(p)) => p,
		_ => return err_resp(id, ERR_INVALID_REQ, "invalid params"),
	};

	match params.uri.as_str() {
		"kern://local/health" => ok(id, resource_content(&params.uri, &resource_health(server))),
		"kern://local/thoughts" => ok(
			id,
			resource_content(&params.uri, &resource_thoughts(server)),
		),
		"kern://local/kerns" => ok(id, resource_content(&params.uri, &resource_kerns(server))),
		"kern://local/descriptors" => ok(
			id,
			resource_content(&params.uri, &resource_descriptors(server)),
		),
		_ => {
			if let Some(tid) = params.uri.strip_prefix("thought://") {
				return ok(
					id,
					resource_content(&params.uri, &resource_thought(server, tid)),
				);
			}
			if let Some(rid) = params.uri.strip_prefix("reason://") {
				return ok(
					id,
					resource_content(&params.uri, &resource_reason(server, rid)),
				);
			}
			err_resp(
				id,
				ERR_NOT_FOUND,
				&format!("unknown resource: {}", params.uri),
			)
		}
	}
}

fn resource_health(server: &Server) -> String {
	serde_json::to_string(&server.health_stats()).unwrap_or_default()
}

fn resource_thoughts(server: &Server) -> String {
	let g = read_recovered(&server.graph);
	let mut all = Vec::new();
	for kern in g.all() {
		for t in kern.entities.values() {
			all.push(serde_json::json!({
				"id": t.id,
				"score": t.score,
				"text": truncate(&t.text(), 200),
				"kern": kern.id,
			}));
		}
	}
	serde_json::to_string(&all).unwrap_or_default()
}

fn resource_kerns(server: &Server) -> String {
	let g = read_recovered(&server.graph);
	let summaries: Vec<serde_json::Value> = g
		.all()
		.iter()
		.map(|k| {
			serde_json::json!({
				"id": k.id,
				"purpose": k.purpose_text,
				"entities": k.entities.len(),
				"reasons": k.reasons.len(),
				"children": k.children.len(),
			})
		})
		.collect();
	serde_json::to_string(&summaries).unwrap_or_default()
}

fn resource_descriptors(server: &Server) -> String {
	let g = read_recovered(&server.graph);
	serde_json::to_string(&g.root.descriptors).unwrap_or_default()
}

fn resource_thought(server: &Server, id: &str) -> String {
	let g = read_recovered(&server.graph);
	match find_entity(&g, id) {
		Some((thought, kern_id)) => {
			let mut edges = Vec::new();
			if let Some(kern) = g.kerns.get(&kern_id) {
				let mut rids = Vec::new();
				if let Some(from_list) = kern.by_from.get(&thought.id) {
					rids.extend(from_list.iter().cloned());
				}
				if let Some(to_list) = kern.by_to.get(&thought.id) {
					rids.extend(to_list.iter().cloned());
				}
				for rid in &rids {
					if let Some(re) = kern.reasons.get(rid) {
						edges.push(serde_json::json!({
							"id": re.id,
							"from": re.from,
							"to": re.to,
							"kind": re.kind as i32,
							"text": re.text,
							"score": re.score,
						}));
					}
				}
			}
			serde_json::to_string(&serde_json::json!({
				"id": thought.id,
				"kind": thought.kind as u8,
				"text": thought.text(),
				"score": thought.score,
				"access_count": thought.access_count.value_i32(),
				"kern": kern_id,
				"edges": edges,
			}))
			.unwrap_or_default()
		}
		None => format!(r#"{{"error":"thought not found: {id}"}}"#),
	}
}

fn resource_reason(server: &Server, id: &str) -> String {
	let g = read_recovered(&server.graph);
	match find_reason(&g, id) {
		Some((reason, _)) => serde_json::to_string(&serde_json::json!({
			"id": reason.id,
			"from": reason.from,
			"to": reason.to,
			"kind": reason.kind as i32,
			"text": reason.text,
			"score": reason.score,
			"traversal_count": reason.traversal_count.value_i32(),
		}))
		.unwrap_or_default(),
		None => format!(r#"{{"error":"reason not found: {id}"}}"#),
	}
}

fn resource_content(uri: &str, text: &str) -> serde_json::Value {
	serde_json::json!({
		"contents": [{
			"uri": uri,
			"mimeType": "application/json",
			"text": text,
		}],
	})
}
