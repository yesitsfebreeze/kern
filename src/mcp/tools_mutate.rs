use serde::Deserialize;

use crate::base::constants::AGENT_SOURCE;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::math::{average_vec, clamp_confidence, cosine, reason_id};
use crate::base::reason::{add_reason, remove_entity, remove_reason};
use crate::base::search::find_entity;
use crate::base::types::{EntityKind, Reason, ReasonKind, Source};
use crate::base::util::explain_relationship_prompt;
use crate::ingest;
use crate::wire::{validate_fact_source, validate_wire_conf, validate_wire_kind};

/// MCP schemas for the mutating tools, co-located with their `tool_ingest` /
/// `tool_link` / `tool_forget` / `tool_degrade` handlers so schema and handler
/// can't drift. Aggregated (in this order) by `tools::tool_definitions`.
pub(crate) fn tool_schemas() -> Vec<serde_json::Value> {
	vec![
		serde_json::json!({
			"name": "ingest",
			"description": "Add text to the knowledge graph.",
			"inputSchema": {
				"type": "object",
				"required": ["text"],
				"properties": {
					"text":       {"type": "string", "description": "text to ingest"},
					"source":     {"type": "string", "description": "source system identifier"},
					"object_id":  {"type": "string", "description": "stable object identifier for update semantics"},
					"section":    {"type": "string", "description": "section within the object"},
					"author":     {"type": "string", "description": "author or origin of the content"},
					"title":      {"type": "string", "description": "human-readable title"},
					"url":        {"type": "string", "description": "URL reference"},
					"conf":       {"type": "number", "description": "confidence weight 0.0-1.0 (default 0.5)"},
					"descriptor": {"type": "string", "description": "Descriptor key for chunking context"},
					"sync":       {"type": "boolean", "description": "block until ingest completes (default false)"},
				},
			},
		}),
		serde_json::json!({
			"name": "link",
			"description": "Create a reason edge between two thoughts.",
			"inputSchema": {
				"type": "object",
				"required": ["from", "to"],
				"properties": {
					"from":   {"type": "string", "description": "source thought ID"},
					"to":     {"type": "string", "description": "target thought ID"},
					"reason": {"type": "string", "description": "reason text (LLM generates if empty)"},
				},
			},
		}),
		serde_json::json!({
			"name": "forget",
			"description": "Remove a thought and cascade-delete its edges. Facts are immune.",
			"inputSchema": {
				"type": "object",
				"required": ["id"],
				"properties": {
					"id": {"type": "string", "description": "thought ID to remove"},
				},
			},
		}),
		serde_json::json!({
			"name": "degrade",
			"description": "Decrease edge scores along the retrieval path for a query.",
			"inputSchema": {
				"type": "object",
				"required": ["query_id"],
				"properties": {
					"query_id": {"type": "string", "description": "thought ID at the end of a bad retrieval path"},
				},
			},
		}),
	]
}

use super::{tool_error, tool_result_json, Server};

#[derive(Deserialize, Default)]
struct IngestArgs {
	#[serde(default)]
	text: String,
	#[serde(default)]
	source: String,
	#[serde(default)]
	object_id: String,
	#[serde(default)]
	section: String,
	#[serde(default)]
	author: String,
	#[serde(default)]
	title: String,
	#[serde(default)]
	url: String,
	#[serde(default)]
	conf: f64,
	#[serde(default)]
	descriptor: String,
	#[serde(default)]
	sync: bool,
	#[serde(default)]
	kind: Option<EntityKind>,
}

/// Wire-boundary validation for an MCP ingest payload, run before any graph
/// access (see docs/kern/safety-architecture.md): confidence range, typed kind,
/// and the fact-tier source-trust guard — an agent caller can mint neither
/// Fact-kind nor Fact-confidence entities. Returns the tool-error text on reject.
fn validate_ingest_wire(p: &IngestArgs) -> Result<(), String> {
	validate_wire_conf(p.conf).map_err(|e| e.to_string())?;
	if let Some(k) = p.kind {
		validate_wire_kind(k).map_err(|e| e.to_string())?;
		if k == EntityKind::Fact {
			validate_fact_source(AGENT_SOURCE).map_err(|e| e.to_string())?;
		}
	}
	if p.conf >= crate::base::constants::FACT_CONFIDENCE {
		validate_fact_source(AGENT_SOURCE).map_err(|e| e.to_string())?;
	}
	Ok(())
}

#[derive(Deserialize)]
struct LinkArgs {
	from: String,
	to: String,
	#[serde(default)]
	reason: String,
}

#[derive(Deserialize)]
struct ForgetArgs {
	id: String,
}

#[derive(Deserialize)]
struct DegradeArgs {
	query_id: String,
}

impl Server {
	pub(crate) fn tool_ingest(&self, args: &serde_json::Value) -> serde_json::Value {
		let p: IngestArgs = match serde_json::from_value(args.clone()) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("invalid arguments: {e}")),
		};
		if p.text.is_empty() {
			return tool_error("text is required");
		}

		// Wire-boundary validation: reject drift-via-mutation before any
		// graph access. See docs/kern/safety-architecture.md.
		if let Err(e) = validate_ingest_wire(&p) {
			return tool_error(&e);
		}

		// MCP callers are agents by construction; clamp against AGENT_SOURCE
		// regardless of what `p.source` claims. The wire `source` string remains
		// descriptive metadata on `Source.system` but cannot escalate the
		// caller to USER_SOURCE trust (which would unlock Fact-tier confidence).
		let (conf, kind) = clamp_confidence(p.conf, AGENT_SOURCE);
		// Map the (legacy) MCP ingest payload to a typed Source variant.
		// Empty `source` collapses to Inline (no scheme); a scheme tag like
		// "file"/"ticket"/"session"/"agent" routes to the matching variant;
		// anything else is treated as a Ticket system descriptor.
		let src = match p.source.as_str() {
			"" | "inline" => Source::Inline {
				hash: p.object_id,
				section: p.section,
			},
			"file" => Source::File {
				path: p.object_id,
				section: p.section,
				title: p.title,
				author: p.author,
				url: p.url,
			},
			"session" => Source::Session {
				session_id: p.object_id,
				section: p.section,
				title: p.title,
			},
			"agent" => Source::Agent {
				agent: p.source.clone(),
				object_id: p.object_id,
				title: p.title,
			},
			other => Source::Ticket {
				system: other.to_string(),
				object_id: p.object_id,
				section: p.section,
				title: p.title,
				author: p.author,
				url: p.url,
			},
		};

		if p.sync {
			let fut = self.worker.run(
				p.text,
				src,
				kind,
				p.descriptor,
				conf,
				ingest::Config {
					dedup_threshold: self.cfg.ingest.dedup_threshold,
					..Default::default()
				},
			);
			let Some(outcome) = crate::llm::block_on_in_place(fut) else {
				return tool_error("no tokio runtime");
			};
			(self.save_fn)();
			return tool_result_json(&serde_json::json!({
				"status": outcome.status.as_str(),
				"doc_id": outcome.doc_id,
				"conf": conf,
				"kind": kind as u8,
				"total_chunks": outcome.total_chunks,
				"embedded_chunks": outcome.embedded_chunks,
				"failed_chunks": outcome.failed_chunks,
				"transient_failures": outcome.transient_failures,
				"permanent_failures": outcome.permanent_failures,
				"message": outcome.message,
			}));
		}

		// Durable ack: persist the payload to the direct spool BEFORE
		// acknowledging, so a daemon exit after the ack loses nothing — the
		// next drain cycle replays it. The in-RAM enqueue path acked "queued"
		// and then held the job only in a 64-slot channel; observed live: a
		// daemon restart vaporized 5 acked ingests. Spool-first only when the
		// drain loop actually runs (capture on + a reason endpoint configured
		// — `spawn_capture` skips the loop otherwise); an undrained spool
		// would be strictly worse than the RAM queue.
		let drain_runs = self.cfg.capture.enabled && !self.cfg.reason_url().is_empty();
		if drain_runs {
			let direct_dir = std::env::current_dir()
				.unwrap_or_else(|_| std::path::PathBuf::from("."))
				.join(&self.cfg.capture.dir)
				.join("direct");
			let job = crate::ingest::direct::DirectJob {
				text: p.text.clone(),
				source: src.clone(),
				kind,
				descriptor: p.descriptor.clone(),
				confidence: conf,
			};
			match crate::ingest::direct::spool_direct(&direct_dir, &job) {
				Ok(doc_id) => {
					return tool_result_json(&serde_json::json!({
						"status": "spooled",
						"doc_id": doc_id,
						"conf": conf,
						"kind": kind as u8,
					}));
				}
				Err(e) => {
					// Fail-open: a spool-write failure (disk full, perms) must
					// not reject knowledge — fall through to the RAM queue and
					// say so in the journal.
					tracing::warn!(
						target: "kern.ingest.direct",
						error = %e,
						"direct spool write failed; falling back to in-RAM enqueue"
					);
				}
			}
		}

		let doc_id = self.worker.enqueue(
			p.text,
			src,
			kind,
			p.descriptor,
			conf,
			ingest::Config {
				dedup_threshold: self.cfg.ingest.dedup_threshold,
				..Default::default()
			},
		);
		tool_result_json(&serde_json::json!({
			"status": "queued",
			"doc_id": doc_id,
			"conf": conf,
			"kind": kind as u8,
		}))
	}

	pub(crate) fn tool_link(&self, args: &serde_json::Value) -> serde_json::Value {
		let p: LinkArgs = match serde_json::from_value(args.clone()) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("invalid arguments: {e}")),
		};

		let g = read_recovered(&self.graph);
		let (from_t, from_kern_id) = match find_entity(&g, &p.from) {
			Some(pair) => pair,
			None => return tool_error(&format!("from thought not found: {}", p.from)),
		};
		let (to_t, _) = match find_entity(&g, &p.to) {
			Some(pair) => pair,
			None => return tool_error(&format!("to thought not found: {}", p.to)),
		};
		drop(g);

		let mut reason_text = p.reason;
		if reason_text.is_empty() {
			if let Some(llm) = &self.llm {
				let prompt = explain_relationship_prompt(&from_t.text(), &to_t.text());
				if let Some(reply) = crate::llm::block_on_in_place(llm.complete(&prompt)) {
					reason_text = reply.unwrap_or_default().trim().to_string();
				}
			}
		}

		let vec = if !reason_text.is_empty() {
			if let Some(llm) = &self.llm {
				crate::llm::block_on_in_place(llm.embed(&reason_text))
					.and_then(Result::ok)
					.unwrap_or_else(|| average_vec(&from_t.vector, &to_t.vector))
			} else {
				average_vec(&from_t.vector, &to_t.vector)
			}
		} else {
			average_vec(&from_t.vector, &to_t.vector)
		};

		let score = cosine(&from_t.vector, &to_t.vector);
		let rid = reason_id(&p.from, &p.to, ReasonKind::Similarity, &reason_text, "");
		let reason = Reason {
			id: rid.clone(),
			from: p.from,
			to: p.to,
			kind: ReasonKind::Similarity,
			text: reason_text,
			vector: vec,
			score,
			..Default::default()
		};

		let mut g = write_recovered(&self.graph);
		if let Some(kern) = g.kerns.get_mut(&from_kern_id) {
			add_reason(kern, reason);
		}
		drop(g);
		(self.save_fn)();

		tool_result_json(&serde_json::json!({"edge_id": rid}))
	}

	pub(crate) fn tool_forget(&self, args: &serde_json::Value) -> serde_json::Value {
		let p: ForgetArgs = match serde_json::from_value(args.clone()) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("invalid arguments: {e}")),
		};

		let mut g = write_recovered(&self.graph);
		let (thought, kern_id) = match find_entity(&g, &p.id) {
			Some(pair) => pair,
			None => return tool_error(&format!("thought not found: {}", p.id)),
		};
		if thought.is_fact() {
			return tool_error("cannot forget a fact");
		}

		let edges_before = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);

		remove_entity(&mut g, &kern_id, &p.id);

		let edges_after = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
		drop(g);
		(self.save_fn)();

		let removed = edges_before - edges_after;
		tool_result_json(&serde_json::json!({"removed_edges": removed}))
	}

	pub(crate) fn tool_degrade(&self, args: &serde_json::Value) -> serde_json::Value {
		let p: DegradeArgs = match serde_json::from_value(args.clone()) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("invalid arguments: {e}")),
		};

		let mut g = write_recovered(&self.graph);
		let (_, kern_id) = match find_entity(&g, &p.query_id) {
			Some(pair) => pair,
			None => return tool_error(&format!("thought not found: {}", p.query_id)),
		};

		let rids: Vec<String> = g
			.kerns
			.get(&kern_id)
			.map(|kern| crate::base::reason::collect_reason_ids(kern, &p.query_id))
			.unwrap_or_default();

		let mut decayed = 0usize;
		for (i, rid) in rids.iter().enumerate() {
			let decay = crate::base::constants::DEGRADE_DECAY_BASE
				* (crate::base::constants::DEGRADE_DECAY_POW).powi(i as i32);

			// One mutable borrow of the kern per edge (was a get-then-get_mut pair).
			let Some(kern) = g.kerns.get_mut(&kern_id) else {
				continue;
			};
			let should_remove = match kern.reasons.get(rid) {
				Some(r) => r.score - decay < crate::base::constants::DEGRADE_MIN_THRESHOLD,
				None => continue,
			};
			if should_remove {
				remove_reason(kern, rid);
			} else if let Some(r) = kern.reasons.get_mut(rid) {
				r.score -= decay;
			}
			decayed += 1;
		}
		drop(g);
		(self.save_fn)();

		tool_result_json(&serde_json::json!({"decayed_edges": decayed}))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use parking_lot::RwLock;
	use std::sync::Arc;

	use crate::base::graph::GraphGnn;
	use crate::base::types::{Entity, Kern};
	use crate::config::Config;
	use crate::llm;
	use crate::mcp::Server;

	fn make_server() -> Server {
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let embedder = llm::Client::new_embed_only("http://127.0.0.1:1", "test");
		let worker = Arc::new(crate::ingest::Worker::new(
			graph.clone(),
			embedder,
			None,
			None,
			None,
		));
		Server {
			graph,
			worker,
			llm: None,
			save_fn: Arc::new(|| {}),
			task_q: None,
			cfg: Arc::new(Config::default()),
			cache: crate::retrieval::cache::QueryCache::default_shared(),
		}
	}

	fn text(out: &serde_json::Value) -> String {
		out["content"][0]["text"].as_str().unwrap_or("").to_string()
	}
	fn body(out: &serde_json::Value) -> serde_json::Value {
		serde_json::from_str(&text(out)).expect("success body is json")
	}
	fn is_error(out: &serde_json::Value) -> bool {
		out
			.get("isError")
			.and_then(|x| x.as_bool())
			.unwrap_or(false)
	}

	fn insert_kern(srv: &Server, kern: Kern) {
		write_recovered(&srv.graph)
			.kerns
			.insert(kern.id.clone(), kern);
	}

	#[tokio::test]
	async fn tool_forget_removes_entity_and_counts_cascaded_edges() {
		let srv = make_server();
		let mut k = Kern::new("kx", "");
		k.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				..Default::default()
			},
		);
		k.entities.insert(
			"b".into(),
			Entity {
				id: "b".into(),
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				id: "a->b".into(),
				from: "a".into(),
				to: "b".into(),
				..Default::default()
			},
		);
		insert_kern(&srv, k);

		let out = srv.tool_forget(&serde_json::json!({ "id": "a" }));
		assert!(!is_error(&out));
		assert_eq!(body(&out)["removed_edges"], 1, "the incident edge cascades");

		let g = read_recovered(&srv.graph);
		assert!(
			!g.kerns.get("kx").unwrap().entities.contains_key("a"),
			"entity is gone"
		);
	}

	#[tokio::test]
	async fn tool_forget_refuses_a_fact() {
		let srv = make_server();
		let mut k = Kern::new("kx", "");
		k.entities.insert(
			"f".into(),
			Entity {
				id: "f".into(),
				kind: EntityKind::Fact,
				..Default::default()
			},
		);
		insert_kern(&srv, k);

		let out = srv.tool_forget(&serde_json::json!({ "id": "f" }));
		assert!(is_error(&out));
		assert!(text(&out).contains("cannot forget a fact"));
	}

	#[tokio::test]
	async fn tool_degrade_decays_survivors_and_reaps_subthreshold() {
		let srv = make_server();
		let mut k = Kern::new("kx", "");
		k.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				id: "a->b".into(),
				from: "a".into(),
				to: "b".into(),
				score: 1.0,
				..Default::default()
			},
		);
		add_reason(
			&mut k,
			Reason {
				id: "a->c".into(),
				from: "a".into(),
				to: "c".into(),
				score: 0.0,
				..Default::default()
			},
		);
		insert_kern(&srv, k);

		let out = srv.tool_degrade(&serde_json::json!({ "query_id": "a" }));
		assert!(!is_error(&out));
		assert_eq!(
			body(&out)["decayed_edges"],
			2,
			"both incident edges visited"
		);

		let g = read_recovered(&srv.graph);
		let kern = g.kerns.get("kx").unwrap();
		assert_eq!(kern.reasons.len(), 1, "the sub-threshold edge is reaped");
		assert!(
			kern.reasons.contains_key("a->b"),
			"the healthy edge survives"
		);
	}

	#[tokio::test]
	async fn tool_link_adds_edge_with_provided_reason_text() {
		let srv = make_server();
		let mut k = Kern::new("kx", "");
		k.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				vector: vec![1.0, 0.0],
				..Default::default()
			},
		);
		k.entities.insert(
			"b".into(),
			Entity {
				id: "b".into(),
				vector: vec![0.0, 1.0],
				..Default::default()
			},
		);
		insert_kern(&srv, k);

		let out =
			srv.tool_link(&serde_json::json!({ "from": "a", "to": "b", "reason": "because related" }));
		assert!(!is_error(&out));
		let edge_id = body(&out)["edge_id"].as_str().expect("edge_id").to_string();

		let g = read_recovered(&srv.graph);
		let r = g
			.kerns
			.get("kx")
			.unwrap()
			.reasons
			.get(&edge_id)
			.expect("edge added to from-kern");
		assert_eq!(
			r.text, "because related",
			"provided reason used verbatim (no LLM configured)"
		);
		assert_eq!((r.from.as_str(), r.to.as_str()), ("a", "b"));
	}

	#[tokio::test]
	async fn tool_link_errors_on_unknown_endpoint() {
		let srv = make_server();
		let out = srv.tool_link(&serde_json::json!({ "from": "nope", "to": "nada", "reason": "x" }));
		assert!(is_error(&out));
		assert!(text(&out).contains("not found"));
	}
}
