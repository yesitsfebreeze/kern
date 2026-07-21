use serde_json::value::RawValue;

use crate::base::search::{find_entity, find_reason};
use crate::base::util::truncate;

use super::{err_resp, ok, Response, Server, ERR_INVALID_REQ, ERR_NOT_FOUND};

pub fn resource_definitions() -> Vec<serde_json::Value> {
	vec![
		serde_json::json!({
			"uri": "kern://local/health",
			"name": "Graph health",
			"description": "Entity/edge counts, tick heat, unnamed count, gravitons",
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
			"description": "All loaded Kerns with gravitons and stats",
			"mimeType": "application/json",
		}),
		serde_json::json!({
			"uri": "kern://local/claim-kinds",
			"name": "Claim kinds",
			"description": "All registered claim kinds (built-ins not included)",
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
		"kern://local/claim-kinds" => ok(
			id,
			resource_content(&params.uri, &resource_claim_kinds(server)),
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

const TOP_THOUGHTS: usize = 50;

fn resource_thoughts(server: &Server) -> String {
	let g = server.graph.read();
	let mut all: Vec<(f64, serde_json::Value)> = Vec::new();
	for kern in g.all() {
		for t in kern.entities.values() {
			// Default-deny: this surface consults no principal, so only an entity
			// carrying no ACL at all is listed here.
			if !t.acl.is_public() {
				continue;
			}
			all.push((
				t.score,
				serde_json::json!({
					"id": t.id,
					"score": t.score,
					"text": truncate(&t.text(), 200),
					"kern": kern.id,
				}),
			));
		}
	}
	all.sort_by(|a, b| {
		let a_id = a.1["id"].as_str().unwrap_or("");
		let b_id = b.1["id"].as_str().unwrap_or("");
		crate::base::util::cmp_rank(a.0, a_id, b.0, b_id)
	});
	let top: Vec<serde_json::Value> = all.into_iter().take(TOP_THOUGHTS).map(|(_, v)| v).collect();
	serde_json::to_string(&top).unwrap_or_default()
}

fn resource_kerns(server: &Server) -> String {
	let g = server.graph.read();
	let summaries: Vec<serde_json::Value> = g
		.all()
		.iter()
		.map(|k| {
			serde_json::json!({
				"id": k.id,
				"graviton": k.graviton_text,
				"entities": k.entities.len(),
				"reasons": k.reasons.len(),
				"children": k.children.len(),
			})
		})
		.collect();
	serde_json::to_string(&summaries).unwrap_or_default()
}

fn resource_claim_kinds(server: &Server) -> String {
	let g = server.graph.read();
	serde_json::to_string(&g.root.claim_kinds).unwrap_or_default()
}

/// What an edge endpoint's ACL says about serving the edge that quotes it.
///
/// Three outcomes and not two, because `find_entity` (`src/base/search.rs:148`)
/// searches only the **resident** kern map — `loaded` is `kerns.get` and `all()`
/// is `kerns.values()`, neither of which sees `unloaded` or the cold tier. So
/// "did not resolve" is emphatically not "does not exist", and treating the two
/// alike is the fail-open case: a scoped row that a GC cold-spill
/// (`src/tick/stigmergy.rs`) or a kern-cap unload (`GraphGnn::unload`) made
/// non-resident is *still alive in the store with its ACL intact* and reads back
/// here as absent. The edge quoting it survives because a kern hosts a reason iff
/// it hosts its `from` (`src/base/reason.rs:78`) — `move_entity` leaves an
/// incoming edge in the *source* kern, and `remove_entity` cascades only within
/// one kern, so nothing ever sweeps it.
enum Endpoint {
	/// Resolved, and carries no ACL.
	Public,
	/// Resolved, and names a scope, user or group.
	Scoped,
	/// Did not resolve. Could be a genuinely dangling id — ordinary here, `to` is
	/// optional in `add_reason` — or a scoped row we simply cannot see.
	Unresolved,
}

fn endpoint(g: &crate::base::graph::GraphGnn, id: &str) -> Endpoint {
	match find_entity(g, id) {
		Some((t, _)) if t.acl.is_public() => Endpoint::Public,
		Some(_) => Endpoint::Scoped,
		None => Endpoint::Unresolved,
	}
}

/// The edge body, with `text` withheld when an endpoint would not clear it.
///
/// `explain_relationship_prompt` (`src/base/util.rs:87`) hands the LLM up to 500
/// chars of BOTH endpoint texts and the reply becomes `reason.text`, so the text
/// belongs to the endpoints, not the edge. Redaction rather than a drop is what
/// keeps default-deny from becoming deny-all: a dangling endpoint is ordinary,
/// and dropping every edge with one would hide a public entity's own structure.
/// The residual is that an unresolved endpoint id is still named — a content
/// hash, so at worst it confirms a guessed text, never discloses one.
fn edge_json(re: &crate::base::types::Reason, text_cleared: bool) -> serde_json::Value {
	serde_json::json!({
		"id": re.id,
		"from": re.from,
		"to": re.to,
		"kind": re.kind as i32,
		"text": if text_cleared { re.text.clone() } else { String::new() },
		"score": re.score,
	})
}

fn resource_thought(server: &Server, id: &str) -> String {
	let g = server.graph.read();
	// A scoped entity reads back exactly like a missing one — telling the two apart
	// would leak the id's existence on the very surface that withholds its text.
	match find_entity(&g, id).filter(|(t, _)| t.acl.is_public()) {
		Some((thought, kern_id)) => {
			let mut edges = Vec::new();
			if let Some(kern) = g.kerns.get(&kern_id) {
				let rids = crate::base::reason::collect_reason_ids(kern, &thought.id);
				for rid in &rids {
					if let Some(re) = kern.reasons.get(rid) {
						// `link` writes edge text by quoting both endpoints, so an edge
						// into a scoped entity is that entity's text under another id.
						// `collect_reason_ids` returns only incident edges, so the far
						// end is `to` when this entity is the `from` and `from` otherwise.
						let other = if re.from == thought.id {
							&re.to
						} else {
							&re.from
						};
						match endpoint(&g, other) {
							Endpoint::Scoped => continue,
							Endpoint::Unresolved => edges.push(edge_json(re, false)),
							Endpoint::Public => edges.push(edge_json(re, true)),
						}
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
	let g = server.graph.read();
	// The edge has no ACL of its own; the entities it hangs between do. Reading it
	// unchecked would be a read of their quoted text through an id that is not
	// theirs, and it is **both** ends that have to clear, not just `from`: the
	// reply to `explain_relationship_prompt` is written from the two texts
	// together, and the response names `to` outright, which is a scoped id on its
	// own.
	let found = find_reason(&g, id).filter(|(reason, _)| {
		// `from` is the entity this edge hangs off. It fails closed on both
		// non-public outcomes: one that did not resolve is not one that said the
		// read was allowed, and it is exactly the endpoint a cold-spill hides.
		matches!(endpoint(&g, &reason.from), Endpoint::Public)
			&& !matches!(endpoint(&g, &reason.to), Endpoint::Scoped)
	});
	match found {
		Some((reason, _)) => {
			// A `to` that did not resolve leaves the text uncleared — same rule as
			// the incident-edge list, and for the same reason.
			let text_cleared = matches!(endpoint(&g, &reason.to), Endpoint::Public);
			serde_json::to_string(&serde_json::json!({
				"id": reason.id,
				"from": reason.from,
				"to": reason.to,
				"kind": reason.kind as i32,
				"text": if text_cleared { reason.text.clone() } else { String::new() },
				"score": reason.score,
				"traversal_count": reason.traversal_count.value_i32(),
			}))
			.unwrap_or_default()
		}
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

#[cfg(test)]
mod tests {
	use super::*;

	use crate::base::reason::add_reason;
	use crate::base::types::{Acl, Entity, Kern, Reason};
	use crate::mcp::Server;

	fn make_server() -> Server {
		crate::test_support::mcp_server()
	}

	fn seed(server: &Server) {
		let mut g = server.graph.write();
		let mut k = Kern::new("kx", "");
		k.entities.insert(
			"e1".into(),
			Entity {
				id: "e1".into(),
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				from: "e1".into(),
				to: "e2".into(),
				id: "r1".into(),
				..Default::default()
			},
		);
		g.kerns.insert("kx".into(), k);
	}

	// Adds the scoped counterpart to `seed`'s public `e1`: `s1` carries an ACL, and
	// is tied to `e1` once in each direction so both endpoint positions are covered.
	fn seed_scoped(server: &Server) {
		let mut g = server.graph.write();
		let k = g.kerns.get_mut("kx").expect("seed() ran first");
		k.entities.insert(
			"s1".into(),
			Entity {
				id: "s1".into(),
				acl: Acl {
					scope: "secret".into(),
					..Default::default()
				},
				..Default::default()
			},
		);
		add_reason(
			k,
			Reason {
				from: "e1".into(),
				to: "s1".into(),
				id: "r2".into(),
				..Default::default()
			},
		);
		add_reason(
			k,
			Reason {
				from: "s1".into(),
				to: "e1".into(),
				id: "r3".into(),
				..Default::default()
			},
		);
	}

	#[tokio::test]
	async fn resource_thoughts_omits_a_scoped_entity() {
		let srv = make_server();
		seed(&srv);
		seed_scoped(&srv);
		let v: serde_json::Value = serde_json::from_str(&resource_thoughts(&srv)).expect("valid json");
		let ids: Vec<&str> = v
			.as_array()
			.expect("a list")
			.iter()
			.filter_map(|t| t["id"].as_str())
			.collect();
		assert!(ids.contains(&"e1"), "the public entity is still listed");
		assert!(!ids.contains(&"s1"), "the scoped entity is withheld");
	}

	#[tokio::test]
	async fn resource_thought_on_a_scoped_entity_reads_as_missing() {
		let srv = make_server();
		seed(&srv);
		seed_scoped(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_thought(&srv, "s1")).expect("error is still valid json");
		assert!(v["error"].as_str().unwrap_or("").contains("not found"));
		assert!(v["text"].is_null(), "no entity text leaks alongside it");
	}

	#[tokio::test]
	async fn resource_thought_drops_edges_touching_a_scoped_entity() {
		let srv = make_server();
		seed(&srv);
		seed_scoped(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_thought(&srv, "e1")).expect("valid json");
		assert_eq!(v["id"], "e1", "the public entity itself still reads");
		let ids: Vec<&str> = v["edges"]
			.as_array()
			.expect("a list")
			.iter()
			.filter_map(|e| e["id"].as_str())
			.collect();
		assert_eq!(
			ids,
			vec!["r1"],
			"r2 (into s1) and r3 (out of s1) are dropped; r1 survives"
		);
	}

	#[tokio::test]
	async fn resource_reason_from_a_scoped_entity_reads_as_missing() {
		let srv = make_server();
		seed(&srv);
		seed_scoped(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_reason(&srv, "r3")).expect("error is still valid json");
		assert!(v["error"].as_str().unwrap_or("").contains("not found"));
		assert!(v["text"].is_null(), "no edge text leaks alongside it");
	}

	// The twin of the `from` test, and not a duplicate of it: gating only `from`
	// left `reason://r2` serving `"to":"s1"` — the scoped id itself — beside text
	// the LLM wrote from up to 500 chars of s1. Public `from`, scoped `to`.
	#[tokio::test]
	async fn resource_reason_to_a_scoped_entity_reads_as_missing() {
		let srv = make_server();
		seed(&srv);
		seed_scoped(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_reason(&srv, "r2")).expect("error is still valid json");
		assert!(v["error"].as_str().unwrap_or("").contains("not found"));
		assert!(v["text"].is_null(), "no edge text leaks alongside it");
		assert!(v["to"].is_null(), "nor the scoped id the edge points at");
	}

	// An id that never resolves is not an id that does not exist: `find_entity`
	// walks only the resident kerns, so a cold-spilled or unloaded scoped row looks
	// exactly like this. The edge stays — a dangling endpoint is ordinary, and
	// dropping it would be deny-all — but the text the LLM wrote from both
	// endpoints does not.
	fn seed_dangling(server: &Server) {
		let mut g = server.graph.write();
		let k = g.kerns.get_mut("kx").expect("seed() ran first");
		add_reason(
			k,
			Reason {
				from: "e1".into(),
				to: "ghost".into(),
				id: "r4".into(),
				text: "e1 and the ghost share a mechanism".into(),
				..Default::default()
			},
		);
	}

	#[tokio::test]
	async fn resource_thought_withholds_edge_text_when_an_endpoint_will_not_resolve() {
		let srv = make_server();
		seed(&srv);
		seed_dangling(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_thought(&srv, "e1")).expect("valid json");
		let r4 = v["edges"]
			.as_array()
			.expect("a list")
			.iter()
			.find(|e| e["id"] == "r4")
			.expect("the edge itself survives — dropping it would be deny-all");
		assert_eq!(
			r4["text"], "",
			"but the text quoting the unseen endpoint does not"
		);
		assert_eq!(r4["to"], "ghost", "the structure is still readable");
	}

	#[tokio::test]
	async fn resource_reason_withholds_text_when_to_will_not_resolve() {
		let srv = make_server();
		seed(&srv);
		seed_dangling(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_reason(&srv, "r4")).expect("valid json");
		assert_eq!(v["id"], "r4", "the edge still reads");
		assert_eq!(v["text"], "", "its text does not");
	}

	#[tokio::test]
	async fn resource_thought_renders_entity_with_its_edges() {
		let srv = make_server();
		seed(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_thought(&srv, "e1")).expect("valid json");
		assert_eq!(v["id"], "e1");
		assert_eq!(v["kern"], "kx");
		assert_eq!(
			v["edges"].as_array().map(|a| a.len()),
			Some(1),
			"the one incident edge"
		);
		assert_eq!(v["edges"][0]["id"], "r1");
	}

	#[tokio::test]
	async fn resource_thought_missing_returns_error_json() {
		let srv = make_server();
		let out = resource_thought(&srv, "nope");
		let v: serde_json::Value = serde_json::from_str(&out).expect("error is still valid json");
		assert!(v["error"].as_str().unwrap_or("").contains("not found"));
	}

	#[tokio::test]
	async fn resource_reason_renders_reason_endpoints() {
		let srv = make_server();
		seed(&srv);
		let v: serde_json::Value =
			serde_json::from_str(&resource_reason(&srv, "r1")).expect("valid json");
		assert_eq!(v["id"], "r1");
		assert_eq!(v["from"], "e1");
		assert_eq!(v["to"], "e2");
	}

	#[tokio::test]
	async fn resource_reason_missing_returns_error_json() {
		let srv = make_server();
		let out = resource_reason(&srv, "nope");
		let v: serde_json::Value = serde_json::from_str(&out).expect("error is still valid json");
		assert!(v["error"].as_str().unwrap_or("").contains("not found"));
	}
}
