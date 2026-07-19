use std::sync::Arc;

use serde_json::Value;
use trnsprt::kern_rpc::{
	GravitonReq, GravitonRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
	DescriptorRes, EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, ForgetReq, ForgetRes,
	HealthRes, IngestReq, IngestRes, KernRpc, LinkReq, LinkRes, ListToolsReq, ListToolsRes,
	NeighborsReq, NeighborsRes, PulseReq, PulseRes, QueryReq, QueryRes, SourceLite,
};
use trnsprt::search::EdgeRef;
use trnsprt::typed::{Channel, JsonEnvelopeCodec, LocalListener};
use trnsprt::McpServer;

use crate::base::types::EntityKind;

#[derive(Clone)]
pub struct KernRpcHandler {
	pub kern: Arc<crate::mcp::Server>,
}

impl KernRpcHandler {
	pub fn new(kern: Arc<crate::mcp::Server>) -> Self {
		Self { kern }
	}
}

fn unwrap_tool_json(envelope: &Value) -> Result<Value, String> {
	if envelope.get("isError").and_then(|v| v.as_bool()) == Some(true) {
		let msg = envelope
			.pointer("/content/0/text")
			.and_then(|v| v.as_str())
			.unwrap_or("kern tool error")
			.to_string();
		return Err(msg);
	}
	let text = envelope
		.pointer("/content/0/text")
		.and_then(|v| v.as_str())
		.unwrap_or("");
	if text.is_empty() {
		return Ok(Value::Null);
	}
	serde_json::from_str(text).map_err(|e| format!("decode tool result: {e}"))
}

fn str_field(v: &Value, key: &str, default: &str) -> String {
	v.get(key)
		.and_then(|x| x.as_str())
		.unwrap_or(default)
		.to_string()
}

fn f32_field(v: &Value, key: &str) -> f32 {
	v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32
}

fn label_from_snippet(snippet: &str) -> String {
	snippet.chars().take(80).collect()
}

fn entity_kind_from_lite(k: EntityKindLite) -> EntityKind {
	match k {
		EntityKindLite::Fact => EntityKind::Fact,
		EntityKindLite::Claim => EntityKind::Claim,
		EntityKindLite::Document => EntityKind::Document,
		EntityKindLite::Question => EntityKind::Question,
		EntityKindLite::Answer => EntityKind::Answer,
		EntityKindLite::Conclusion => EntityKind::Conclusion,
		// Superseded is a status, not a kind — map to next-best; callers set status separately.
		EntityKindLite::Superseded => EntityKind::Claim,
	}
}

fn parse_status_label(s: &str) -> EntityStatusLite {
	if s == "superseded" {
		EntityStatusLite::Superseded
	} else {
		EntityStatusLite::Active
	}
}

fn entity_kind_to_lite(k: EntityKind) -> EntityKindLite {
	match k {
		EntityKind::Fact => EntityKindLite::Fact,
		EntityKind::Claim => EntityKindLite::Claim,
		EntityKind::Document => EntityKindLite::Document,
		EntityKind::Question => EntityKindLite::Question,
		EntityKind::Answer => EntityKindLite::Answer,
		EntityKind::Conclusion => EntityKindLite::Conclusion,
	}
}

// Deliberately NOT 1:1: a semantic projection of ReasonKind, not a cast.
fn edge_kind_from_reason_int(kind: i32) -> EdgeKind {
	match kind {
		0 => EdgeKind::References,   // Similarity — generic semantic relatedness
		1 => EdgeKind::Derives,      // Provenance — sourced/derived from
		2 => EdgeKind::Answers,      // Question — resolved to its answer entity
		3 => EdgeKind::PartOf,       // Spawn — sub-cluster under its parent kern
		4 => EdgeKind::Consolidates, // Supersedes — newer folds in / replaces older
		5 => EdgeKind::Supports,     // Ratification — confirms / upholds
		6 => EdgeKind::References,   // Rephrase — restatement, no stronger wire kind
		_ => EdgeKind::References,   // unknown / future discriminant
	}
}

fn edge_kind_tag(k: EdgeKind) -> &'static str {
	match k {
		EdgeKind::Answers => "answers",
		EdgeKind::Supports => "supports",
		EdgeKind::Contradicts => "contradicts",
		EdgeKind::Extends => "extends",
		EdgeKind::Requires => "requires",
		EdgeKind::References => "references",
		EdgeKind::Derives => "derives",
		EdgeKind::Instances => "instances",
		EdgeKind::PartOf => "part_of",
		EdgeKind::Consolidates => "consolidates",
	}
}

impl KernRpc for KernRpcHandler {
	fn query(&self, req: QueryReq) -> impl ::core::future::Future<Output = QueryRes> + Send {
		let kern = self.kern.clone();
		async move {
			let mode = if req.mode.is_empty() {
				"hybrid".to_string()
			} else {
				req.mode.clone()
			};
			let args = serde_json::json!({
					"text": req.text,
					"k": req.k,
					"mode": mode,
					"answer": req.answer,
					"kind": req.kind,
					"scheme": if req.source.is_empty() { Value::Null } else { Value::String(req.source.clone()) },
			});
			let env = kern.tool_query(&args);
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(_) => return QueryRes::default(),
			};
			let answer = str_field(&payload, "answer", "");
			let mut hits: Vec<EntityRef> = Vec::new();
			if let Some(arr) = payload.get("entities").and_then(|v| v.as_array()) {
				for e in arr {
					let id = str_field(e, "id", "");
					if id.is_empty() {
						continue;
					}
					let score = f32_field(e, "score");
					let snippet = str_field(e, "text", "");
					let label = label_from_snippet(&snippet);
					// Envelope echoes kind/scheme/status off the matched Entity (no second
					// graph read); defaults cover older envelopes / external clients.
					let kind = e
						.get("kind")
						.and_then(|v| v.as_str())
						.and_then(EntityKindLite::from_label)
						.unwrap_or(EntityKindLite::Claim);
					let scheme = str_field(e, "scheme", "inline");
					let status = e
						.get("status")
						.and_then(|v| v.as_str())
						.map(parse_status_label)
						.unwrap_or(EntityStatusLite::Active);
					let edges: Vec<EdgeRef> = e
						.get("edges")
						.and_then(|v| v.as_array())
						.map(|arr| {
							arr
								.iter()
								.filter_map(|edge| {
									let text = str_field(edge, "text", "");
									if text.is_empty() {
										return None;
									}
									Some(EdgeRef {
										from: str_field(edge, "from", ""),
										to: str_field(edge, "to", ""),
										kind: edge_kind_from_reason_int(
											edge.get("kind").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
										),
										text,
										score: f32_field(edge, "score"),
									})
								})
								.collect()
						})
						.unwrap_or_default();
					hits.push(EntityRef {
						id,
						kind,
						status,
						scheme,
						label,
						snippet,
						score,
						edges,
					});
				}
			}

			QueryRes {
				hits,
				answer,
				fresh: true,
			}
		}
	}

	fn ingest(&self, req: IngestReq) -> impl ::core::future::Future<Output = IngestRes> + Send {
		let kern = self.kern.clone();
		async move {
			let kind = entity_kind_from_lite(req.kind);
			let (source_tag, object_id, section, title, author, url) = match &req.source {
				SourceLite::File {
					path,
					section,
					title,
					author,
					url,
				} => (
					"file",
					path.clone(),
					section.clone(),
					title.clone(),
					author.clone(),
					url.clone(),
				),
				SourceLite::Ticket {
					system,
					object_id,
					section,
					title,
					author,
					url,
				} => (
					system.as_str(),
					object_id.clone(),
					section.clone(),
					title.clone(),
					author.clone(),
					url.clone(),
				),
				SourceLite::Session {
					session_id,
					section,
					title,
				} => (
					"session",
					session_id.clone(),
					section.clone(),
					title.clone(),
					String::new(),
					String::new(),
				),
				SourceLite::Agent {
					agent,
					object_id,
					title,
				} => (
					"agent",
					object_id.clone(),
					String::new(),
					title.clone(),
					agent.clone(),
					String::new(),
				),
				SourceLite::Inline { hash, section } => (
					"inline",
					hash.clone(),
					section.clone(),
					String::new(),
					String::new(),
					String::new(),
				),
			};
			let descriptor = req.descriptor.clone().unwrap_or_default();
			let args = serde_json::json!({
					"text": req.text,
					"source": source_tag,
					"object_id": object_id,
					"section": section,
					"author": author,
					"title": title,
					"url": url,
					"conf": req.conf,
					"descriptor": descriptor,
					"sync": req.sync,
					"kind": kind as u8,
			});
			let env = kern.tool_ingest(&args);
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(msg) => {
					return IngestRes {
						entity_id: String::new(),
						status: "rejected".into(),
						message: msg,
					};
				}
			};
			let entity_id = str_field(&payload, "doc_id", "");
			let status = str_field(&payload, "status", "queued");
			let message = str_field(&payload, "message", "");

			IngestRes {
				entity_id,
				status,
				message,
			}
		}
	}

	fn link(&self, req: LinkReq) -> impl ::core::future::Future<Output = LinkRes> + Send {
		let kern = self.kern.clone();
		async move {
			let reason_text = if req.text.is_empty() {
				format!("[{}]", edge_kind_tag(req.reason_kind))
			} else {
				req.text.clone()
			};
			let args = serde_json::json!({
					"from": req.from_id,
					"to": req.to_id,
					"reason": reason_text,
			});
			let env = kern.tool_link(&args);
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(_) => return LinkRes::default(),
			};
			let reason_id = payload
				.get("edge_id")
				.or_else(|| payload.get("id"))
				.or_else(|| payload.get("reason_id"))
				.and_then(|v| v.as_str())
				.unwrap_or("")
				.to_string();
			LinkRes { reason_id }
		}
	}

	fn neighbors(
		&self,
		req: NeighborsReq,
	) -> impl ::core::future::Future<Output = NeighborsRes> + Send {
		let kern = self.kern.clone();
		async move {
			// `req.depth` is intentionally not honored — only depth-1 is implemented.
			let args = serde_json::json!({ "id": req.entity_id });
			let env = kern.tool_query(&args);
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(_) => return NeighborsRes::default(),
			};
			// Cap BEFORE the per-id detail fetch: edge count is data-controlled, so
			// an uncapped detail loop is an N+1 amplification vector.
			let neighbour_ids = collect_neighbour_ids(&payload, &req.entity_id, MAX_NEIGHBORS);
			let mut neighbors = Vec::with_capacity(neighbour_ids.len());
			for id in neighbour_ids {
				let detail_args = serde_json::json!({ "id": id });
				let env = kern.tool_query(&detail_args);
				let detail = match unwrap_tool_json(&env) {
					Ok(v) => v,
					Err(_) => continue,
				};
				let snippet = str_field(&detail, "text", "");
				let score = f32_field(&detail, "score");
				let kind_u8 = detail.get("kind").and_then(|v| v.as_u64()).unwrap_or(1) as u8;
				let kind_internal = match kind_u8 {
					0 => EntityKind::Fact,
					2 => EntityKind::Document,
					3 => EntityKind::Question,
					4 => EntityKind::Answer,
					5 => EntityKind::Conclusion,
					_ => EntityKind::Claim,
				};
				let (_, scheme, status) = lookup_kind_scheme_status(&kern, &id);
				let label = label_from_snippet(&snippet);
				neighbors.push(EntityRef {
					id,
					kind: entity_kind_to_lite(kind_internal),
					status,
					scheme,
					label,
					snippet,
					score,
					edges: vec![],
				});
			}
			NeighborsRes { neighbors }
		}
	}

	fn forget(&self, req: ForgetReq) -> impl ::core::future::Future<Output = ForgetRes> + Send {
		let kern = self.kern.clone();
		async move {
			let args = serde_json::json!({ "id": req.id });
			let env = kern.tool_forget(&args);
			// tool_forget emits only `removed_edges` on success; success-implies-removed.
			let removed = unwrap_tool_json(&env).is_ok();
			ForgetRes { removed }
		}
	}

	fn degrade(&self, req: DegradeReq) -> impl ::core::future::Future<Output = DegradeRes> + Send {
		let kern = self.kern.clone();
		async move {
			// Preserve legacy memory_rpc id → query_id remap for tool_degrade.
			let args = serde_json::json!({ "query_id": req.id });
			let env = kern.tool_degrade(&args);
			// tool_degrade emits only `decayed_edges`; success-implies-applied.
			let applied = unwrap_tool_json(&env).is_ok();
			DegradeRes { applied }
		}
	}

	fn health(&self) -> impl ::core::future::Future<Output = HealthRes> + Send {
		let kern = self.kern.clone();
		async move {
			let env = kern.tool_health();
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(_) => return HealthRes::default(),
			};
			let kerns = payload.get("kerns").and_then(|v| v.as_u64()).unwrap_or(0);
			let entities = payload
				.get("entities")
				.and_then(|v| v.as_u64())
				.unwrap_or(0);
			let data_dir = payload
				.get("data_dir")
				.and_then(|v| v.as_str())
				.unwrap_or("")
				.to_string();
			HealthRes {
				ok: true,
				data_dir,
				kerns,
				entities,
			}
		}
	}

	fn graviton(&self, req: GravitonReq) -> impl ::core::future::Future<Output = GravitonRes> + Send {
		let kern = self.kern.clone();
		async move {
			let args = serde_json::json!({
					"action": req.action,
					"name": req.name,
					"text": req.text,
			});
			let result = kern.tool_graviton(&args);
			GravitonRes {
				result: result.to_string(),
			}
		}
	}

	fn descriptor(
		&self,
		req: DescriptorReq,
	) -> impl ::core::future::Future<Output = DescriptorRes> + Send {
		let kern = self.kern.clone();
		async move {
			let args = serde_json::json!({
					"action": req.action,
					"name": req.name,
					"description": req.description,
			});
			let _ = kern.tool_descriptor(&args);
			DescriptorRes::default()
		}
	}

	fn pulse(&self, req: PulseReq) -> impl ::core::future::Future<Output = PulseRes> + Send {
		let kern = self.kern.clone();
		async move {
			let args = serde_json::json!({ "strength": req.strength });
			let _ = kern.tool_pulse(&args);
			PulseRes::default()
		}
	}

	fn call_tool(
		&self,
		req: CallToolReq,
	) -> impl ::core::future::Future<Output = CallToolRes> + Send {
		let kern = self.kern.clone();
		async move {
			// Forward to the single MCP `call_tool` dispatcher so the proxy relays any
			// `tools/call` over kern.sock without enumerating every tool.
			let envelope = match McpServer::call_tool(&*kern, &req.name, &req.args) {
				Ok(tr) => serde_json::json!({
						"content": tr.content,
						"isError": tr.is_error,
				}),
				Err(e) => serde_json::json!({
						"content": [{
								"type": "text",
								"text": format!("kern_rpc::call_tool: {e}"),
						}],
						"isError": true,
				}),
			};
			CallToolRes { envelope }
		}
	}

	fn list_tools(
		&self,
		_req: ListToolsReq,
	) -> impl ::core::future::Future<Output = ListToolsRes> + Send {
		let kern = self.kern.clone();
		async move {
			// Serialise the live `tools/list` to raw schemas so the proxy advertises
			// exactly what we expose.
			let tools = McpServer::tools_list(&*kern)
				.iter()
				.filter_map(|s| serde_json::to_value(s).ok())
				.collect();
			ListToolsRes { tools }
		}
	}
}

// Caps the detail loop against the entity's data-controlled degree (N+1 guard).
const MAX_NEIGHBORS: usize = 64;

fn collect_neighbour_ids(payload: &serde_json::Value, entity_id: &str, max: usize) -> Vec<String> {
	let mut ids: Vec<String> = Vec::new();
	let Some(edges) = payload.get("edges").and_then(|v| v.as_array()) else {
		return ids;
	};
	for e in edges {
		if ids.len() >= max {
			break;
		}
		let from = e.get("from").and_then(|v| v.as_str()).unwrap_or("");
		let to = e.get("to").and_then(|v| v.as_str()).unwrap_or("");
		let other = if from == entity_id { to } else { from };
		if other.is_empty() || other == entity_id {
			continue;
		}
		if !ids.iter().any(|x| x == other) {
			ids.push(other.to_string());
		}
	}
	ids
}

fn lookup_kind_scheme_status(
	server: &Arc<crate::mcp::Server>,
	id: &str,
) -> (EntityKindLite, String, EntityStatusLite) {
	use crate::base::search::find_entity;
	let g = crate::base::locks::read_recovered(&server.graph);
	match find_entity(&g, id) {
		Some((ent, _)) => (
			entity_kind_to_lite(ent.kind),
			ent.source.scheme().to_string(),
			if ent.is_superseded() {
				EntityStatusLite::Superseded
			} else {
				EntityStatusLite::Active
			},
		),
		None => (
			EntityKindLite::Claim,
			"inline".into(),
			EntityStatusLite::Active,
		),
	}
}

pub async fn serve_kern_rpc_loop(mut listener: LocalListener, handler: KernRpcHandler) {
	loop {
		let adapter = match listener.accept().await {
			Ok(a) => a,
			Err(e) => {
				tracing::warn!(target: "kern.kern_rpc", error = %e, "accept");
				continue;
			}
		};
		let handler = handler.clone();
		tokio::spawn(async move {
			let channel = Channel::new(adapter, JsonEnvelopeCodec::new());
			if let Err(e) = trnsprt::kern_rpc::serve_kern_rpc(channel, handler).await {
				tracing::warn!(target: "kern.kern_rpc", error = %e, "serve loop");
			}
		});
	}
}

#[cfg(test)]
mod envelope_parse_tests {
	use super::*;

	#[test]
	fn from_label_round_trips_every_kind() {
		for k in [
			EntityKind::Fact,
			EntityKind::Claim,
			EntityKind::Document,
			EntityKind::Question,
			EntityKind::Answer,
			EntityKind::Conclusion,
		] {
			let label = k.as_str();
			let lite = EntityKindLite::from_label(label).expect("known label parses");
			assert_eq!(lite, entity_kind_to_lite(k), "round-trip for {label}");
		}
	}

	#[test]
	fn from_label_unknown_returns_none() {
		assert!(EntityKindLite::from_label("").is_none());
		assert!(EntityKindLite::from_label("bogus").is_none());
		assert!(EntityKindLite::from_label("superseded").is_none());
	}

	#[test]
	fn parse_status_label_recognises_superseded() {
		assert_eq!(
			parse_status_label("superseded"),
			EntityStatusLite::Superseded
		);
	}

	#[test]
	fn parse_status_label_defaults_to_active() {
		assert_eq!(parse_status_label("active"), EntityStatusLite::Active);
		assert_eq!(parse_status_label(""), EntityStatusLite::Active);
		assert_eq!(parse_status_label("garbage"), EntityStatusLite::Active);
	}

	#[test]
	fn edge_kind_decode_projects_each_reason_kind() {
		use crate::base::types::ReasonKind;
		let cases = [
			(ReasonKind::Similarity, EdgeKind::References),
			(ReasonKind::Provenance, EdgeKind::Derives),
			(ReasonKind::Question, EdgeKind::Answers),
			(ReasonKind::Spawn, EdgeKind::PartOf),
			(ReasonKind::Supersedes, EdgeKind::Consolidates),
			(ReasonKind::Ratification, EdgeKind::Supports),
			(ReasonKind::Rephrase, EdgeKind::References),
		];
		for (rk, expected) in cases {
			assert_eq!(
				edge_kind_from_reason_int(rk as i32),
				expected,
				"ReasonKind {rk:?} must project to {expected:?}",
			);
		}
		assert_ne!(
			edge_kind_from_reason_int(ReasonKind::Question as i32),
			EdgeKind::References
		);
	}

	#[test]
	fn edge_kind_decode_unknown_falls_back_to_references() {
		assert_eq!(edge_kind_from_reason_int(-1), EdgeKind::References);
		assert_eq!(edge_kind_from_reason_int(99), EdgeKind::References);
	}
}

#[cfg(test)]
mod neighbors_cap_tests {
	use super::*;
	use serde_json::json;

	fn payload_with_edges(center: &str, others: &[&str]) -> serde_json::Value {
		let edges: Vec<_> = others
			.iter()
			.map(|o| json!({ "from": center, "to": o }))
			.collect();
		json!({ "edges": edges })
	}

	#[test]
	fn caps_high_degree_fan_out() {
		let names: Vec<String> = (0..500).map(|i| format!("n{i}")).collect();
		let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
		let payload = payload_with_edges("center", &refs);
		let ids = collect_neighbour_ids(&payload, "center", MAX_NEIGHBORS);
		assert_eq!(ids.len(), MAX_NEIGHBORS, "fan-out capped at MAX_NEIGHBORS");
	}

	#[test]
	fn excludes_self_and_empty_and_dedups() {
		let payload = json!({ "edges": [
				{ "from": "c", "to": "a" },
				{ "from": "b", "to": "c" },
				{ "from": "c", "to": "c" },
				{ "from": "c", "to": "" },
				{ "from": "c", "to": "a" },
		]});
		let ids = collect_neighbour_ids(&payload, "c", MAX_NEIGHBORS);
		assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
	}

	#[test]
	fn missing_edges_yields_empty() {
		let ids = collect_neighbour_ids(&json!({}), "c", MAX_NEIGHBORS);
		assert!(ids.is_empty());
	}
}
