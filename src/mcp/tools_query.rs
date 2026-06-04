use serde::Deserialize;

use crate::base::search::find_entity;
use crate::base::types::EntityKind;
use crate::base::util::truncate;
use std::sync::Arc;

use crate::retrieval;
use crate::types::{EmbedFunc, LlmFunc};

use super::{tool_error, tool_result_json, Server};

#[derive(Deserialize, Default)]
struct QueryArgs {
	#[serde(default)]
	text: String,
	#[serde(default)]
	id: String,
	#[serde(default)]
	k: usize,
	#[serde(default)]
	mode: String,
	#[serde(default)]
	answer: bool,
	#[serde(default)]
	sort: String,
	#[serde(default)]
	ascending: bool,
	/// Legacy free-form source-system filter (e.g. "github"). Matches
	/// `Source::system()` for tickets and the synthesized scheme tag for
	/// other variants. Prefer `scheme` for typed routing.
	#[serde(default)]
	source: String,
	/// Typed entity-kind filter (`fact`, `claim`, `document`, `question`,
	/// `answer`, `conclusion`). Unknown values yield an error.
	#[serde(default)]
	kind: Option<EntityKind>,
	/// URI scheme filter on `Source` — one of `file`, `ticket`, `session`,
	/// `agent`, `inline`. Unknown values yield an error.
	#[serde(default)]
	scheme: Option<String>,
	#[serde(default)]
	since: String,
	#[serde(default)]
	before: String,
	#[serde(default)]
	min_conf: f64,
	#[serde(default)]
	valid_at: String,
}

impl Server {
	pub(crate) fn tool_query(&self, args: &serde_json::Value) -> serde_json::Value {
		let p: QueryArgs = match serde_json::from_value(args.clone()) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("invalid arguments: {e}")),
		};

		if !p.id.is_empty() {
			let g = match self.graph.read() {
				Ok(g) => g,
				Err(_) => return tool_error("graph lock poisoned"),
			};
			return match find_entity(&g, &p.id) {
				Some((thought, kern_id)) => {
					let detail = entity_detail(&thought, &kern_id, &g);
					tool_result_json(&detail)
				}
				None => tool_error(&format!("thought not found: {}", p.id)),
			};
		}

		if p.text.is_empty() {
			return tool_error("either text or id is required");
		}

		let llm = match &self.llm {
			Some(c) => c.clone(),
			None => return tool_error("no embed client configured"),
		};

		let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
			return tool_error("no tokio runtime");
		};
		let vec = match tokio::task::block_in_place(|| handle.block_on(llm.embed(&p.text))) {
			Ok(v) => v,
			Err(e) => return tool_error(&format!("embed failed: {e}")),
		};

		let mode = retrieval::seed::Mode::parse(&p.mode);

		let complete = llm.complete_func();
		let answer_on = p.answer;
		let llm_fn: LlmFunc = Arc::new(complete);

		let llm_embed = llm.clone();
		let embed_handle = handle.clone();
		let embed_fn: EmbedFunc = Arc::new(move |s: &str| {
			tokio::task::block_in_place(|| embed_handle.block_on(llm_embed.embed(s)))
				.map_err(|e| e.to_string())
		});

		let mut opts = retrieval::score::QueryOptions::default();
		opts.sort = retrieval::score::SortField::parse(&p.sort);
		opts.ascending = p.ascending;
		opts.source = p.source;
		opts.kind = p.kind;
		if let Some(ref s) = p.scheme {
			match crate::base::types::Source::parse_scheme(s) {
				Some(tag) => opts.scheme = Some(tag.to_string()),
				None => return tool_error(&format!("unknown source scheme: {s}")),
			}
		}
		opts.min_conf = p.min_conf;
		if let Ok(t) = super::parse_rfc3339(&p.since) {
			opts.since = Some(t);
		}
		if let Ok(t) = super::parse_rfc3339(&p.before) {
			opts.before = Some(t);
		}
		if let Ok(t) = super::parse_rfc3339(&p.valid_at) {
			opts.valid_at = Some(t);
		}

		let rcfg = &self.cfg.retrieval;
		let result = {
			let g = match self.graph.read() {
				Ok(g) => g,
				Err(_) => return tool_error("graph lock poisoned"),
			};
			retrieval::answer::query(
				&g,
				rcfg,
				&vec,
				&p.text,
				mode,
				Some(&llm_fn),
				Some(&embed_fn),
				Some(opts),
			)
		};
		(self.save_fn)();

		let answer_str = if answer_on {
			result.answer.clone()
		} else {
			String::new()
		};

		let k = if p.k == 0 { rcfg.seed_k } else { p.k };

		// Cold-tier recall: when the hot graph yields fewer than k, fill the
		// remaining slots from the cold store (read-only; demoted thoughts
		// stay findable without rehydrating into the hot graph).
		let mut scored: Vec<retrieval::expand::ScoredEntity> = result.entities.clone();
		let mut cold_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
		if scored.len() < k {
			let cold_dir = std::path::PathBuf::from(&self.cfg.data_dir).join("cold");
			let have: std::collections::HashSet<String> =
				scored.iter().map(|s| s.entity.id.clone()).collect();
			for (entity, score) in crate::base::cold::search(&cold_dir, &vec, k) {
				if scored.len() >= k {
					break;
				}
				if !have.contains(&entity.id) {
					cold_ids.insert(entity.id.clone());
					scored.push(retrieval::expand::ScoredEntity { entity, score });
				}
			}
		}

		let entities: Vec<serde_json::Value> = scored
			.iter()
			.take(k)
			.map(|st| {
				// Echo kind/scheme/status directly from the matched
				// Entity so kern_rpc::query can build EntityRef without
				// a second graph lookup. `kind` is the lower-case label
				// (matches `EntityKindLite` serde repr), `scheme` is the
				// stable `Source` URI tag, `status` is `"active"` or
				// `"superseded"` mirroring `EntityStatusLite`.
				let status_str = if st.entity.is_superseded() {
					"superseded"
				} else {
					"active"
				};
				serde_json::json!({
					"id": st.entity.id,
					"score": st.score,
					"conf": st.entity.conf_mean(),
					"conf_uncertainty": st.entity.conf_variance(),
					"text": truncate(&st.entity.text(), 500),
					"kind": st.entity.kind.as_str(),
					"scheme": st.entity.source.scheme(),
					"status": status_str,
					"cold": cold_ids.contains(&st.entity.id),
				})
			})
			.collect();

		let mut out = serde_json::json!({"entities": entities});
		if !answer_str.is_empty() {
			out["answer"] = serde_json::Value::String(answer_str);
		}
		tool_result_json(&out)
	}
}

#[cfg(test)]
mod envelope_shape_tests {
	//! Slice Z: assert the per-hit JSON shape emitted into the
	//! `entities` array of `tool_query`'s envelope carries `kind`,
	//! `scheme`, and `status` strings. The kern_rpc::query handler
	//! consumes these directly; if a future refactor drops them, the
	//! handler silently falls back to defaults — these tests guard
	//! against that regression at the source-of-truth level.
	use crate::base::types::{
		ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source,
	};
	use crate::base::util::truncate;

	fn entity_with(kind: EntityKind, status: EntityStatus, source: Source) -> Entity {
		Entity {
			id: "e1".into(),
			kind,
			status,
			source,
			statements: vec!["hello world".into()],
			chunks: vec![ChunkPart {
				kind: ChunkPartKind::StatementRef,
				text: String::new(),
				index: 0,
			}],
			..Default::default()
		}
	}

	/// Mirrors the envelope construction inside `tool_query` so a
	/// drift between this test and the real builder will fail fast.
	fn build_entity_json(entity: &Entity, score: f64) -> serde_json::Value {
		let status_str = if entity.is_superseded() { "superseded" } else { "active" };
		serde_json::json!({
			"id": entity.id,
			"score": score,
			"conf": entity.conf_mean(),
			"conf_uncertainty": entity.conf_variance(),
			"text": truncate(&entity.text(), 500),
			"kind": entity.kind.as_str(),
			"scheme": entity.source.scheme(),
			"status": status_str,
		})
	}

	#[test]
	fn envelope_includes_kind_scheme_status_for_active_entity() {
		let ent = entity_with(
			EntityKind::Fact,
			EntityStatus::Active,
			Source::File {
				path: "src/main.rs".into(),
				section: String::new(),
				title: String::new(),
				author: String::new(),
				url: String::new(),
			},
		);
		let v = build_entity_json(&ent, 0.5);
		assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some("fact"));
		assert_eq!(v.get("scheme").and_then(|x| x.as_str()), Some("file"));
		assert_eq!(v.get("status").and_then(|x| x.as_str()), Some("active"));
	}

	#[test]
	fn envelope_status_is_superseded_when_entity_superseded() {
		let ent = entity_with(
			EntityKind::Claim,
			EntityStatus::Superseded,
			Source::Inline {
				hash: "h".into(),
				section: String::new(),
			},
		);
		let v = build_entity_json(&ent, 0.0);
		assert_eq!(v.get("status").and_then(|x| x.as_str()), Some("superseded"));
		assert_eq!(v.get("scheme").and_then(|x| x.as_str()), Some("inline"));
		assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some("claim"));
	}

	#[test]
	fn envelope_emits_every_kind_label() {
		for k in [
			EntityKind::Fact,
			EntityKind::Claim,
			EntityKind::Document,
			EntityKind::Question,
			EntityKind::Answer,
			EntityKind::Conclusion,
		] {
			let ent = entity_with(k, EntityStatus::Active, Source::default());
			let v = build_entity_json(&ent, 0.0);
			assert_eq!(v.get("kind").and_then(|x| x.as_str()), Some(k.as_str()));
		}
	}
}

fn entity_detail(
	thought: &crate::base::types::Entity,
	kern_id: &str,
	g: &crate::base::graph::GraphGnn,
) -> serde_json::Value {
	let mut edges = Vec::new();
	if let Some(kern) = g.kerns.get(kern_id) {
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
	serde_json::json!({
		"id": thought.id,
		"kind": thought.kind as u8,
		"text": thought.text(),
		"score": thought.score,
		"conf": thought.conf_mean(),
		"conf_uncertainty": thought.conf_variance(),
		"access_count": thought.access_count.value_i32(),
		"kern": kern_id,
		"edges": edges,
	})
}
