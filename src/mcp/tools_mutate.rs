use serde::Deserialize;

use crate::base::constants::AGENT_SOURCE;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::math::{average_vec, clamp_confidence, cosine, reason_id};
use crate::base::reason::{add_reason, remove_reason, remove_entity};
use crate::base::search::find_entity;
use crate::base::types::{Reason, ReasonKind, Source, EntityKind};
use crate::base::util::explain_relationship_prompt;
use crate::ingest;
use crate::wire::{validate_fact_source, validate_wire_conf, validate_wire_kind};

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
		if let Err(e) = validate_wire_conf(p.conf) {
			return tool_error(&e.to_string());
		}
		if let Some(k) = p.kind {
			if let Err(e) = validate_wire_kind(k) {
				return tool_error(&e.to_string());
			}
			if k == EntityKind::Fact {
				if let Err(e) = validate_fact_source(AGENT_SOURCE) {
					return tool_error(&e.to_string());
				}
			}
		}
		if p.conf >= crate::base::constants::FACT_CONFIDENCE {
			if let Err(e) = validate_fact_source(AGENT_SOURCE) {
				return tool_error(&e.to_string());
			}
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
		#[derive(Deserialize)]
		struct LinkArgs {
			from: String,
			to: String,
			#[serde(default)]
			reason: String,
		}

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
		#[derive(Deserialize)]
		struct ForgetArgs {
			id: String,
		}

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
		#[derive(Deserialize)]
		struct DegradeArgs {
			query_id: String,
		}

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

			let should_remove = if let Some(kern) = g.kerns.get(&kern_id) {
				if let Some(r) = kern.reasons.get(rid) {
					r.score - decay < crate::base::constants::DEGRADE_MIN_THRESHOLD
				} else {
					continue;
				}
			} else {
				continue;
			};

			if should_remove {
				if let Some(kern) = g.kerns.get_mut(&kern_id) {
					remove_reason(kern, rid);
				}
			} else if let Some(kern) = g.kerns.get_mut(&kern_id) {
				if let Some(r) = kern.reasons.get_mut(rid) {
					r.score -= decay;
				}
			}
			decayed += 1;
		}
		drop(g);
		(self.save_fn)();

		tool_result_json(&serde_json::json!({"decayed_edges": decayed}))
	}
}
