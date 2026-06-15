use serde::Deserialize;

use crate::base::search::find_entity;
use crate::base::types::EntityKind;
use crate::base::util::truncate;
use std::sync::Arc;

use crate::retrieval;
use crate::types::{EmbedFunc, LlmFunc};

/// MCP schema for the `query` tool, co-located with its `tool_query` handler so
/// the two cannot silently drift. Aggregated by `tools::tool_definitions`.
pub(crate) fn tool_schemas() -> Vec<serde_json::Value> {
	vec![serde_json::json!({
		"name": "query",
		"description": "Search the knowledge graph. Returns scored thoughts and optionally an LLM answer. Requires at least one of `text` (semantic/lexical search) or `id` (direct lookup).",
		"inputSchema": {
			"type": "object",
			// Mirrors tool_query's runtime guard ("either text or id is required"):
			// a filter-only call with neither is rejected, so the schema says so too.
			"anyOf": [
				{"required": ["text"]},
				{"required": ["id"]},
			],
			"properties": {
				"text":      {"type": "string", "description": "search query text"},
				"id":        {"type": "string", "description": "thought ID for direct lookup"},
				"k":         {"type": "integer", "description": "number of results (default 5)"},
				"mode":      {"type": "string", "enum": ["content", "reason", "hybrid"], "description": "retrieval mode (default hybrid)"},
				"answer":    {"type": "boolean", "description": "synthesize an LLM answer"},
				"sort":      {"type": "string", "enum": ["", "date", "access", "confidence"], "description": "sort key"},
				"ascending": {"type": "boolean", "description": "sort ascending (default false)"},
				"source":    {"type": "string", "description": "filter by source system"},
				"kind":      {"type": "string", "enum": ["", "normal", "fact", "document"], "description": "filter by thought kind"},
				"since":     {"type": "string", "description": "ISO8601 timestamp; only include thoughts at or after this time"},
				"before":    {"type": "string", "description": "ISO8601 timestamp; only include thoughts before this time"},
				"min_conf":  {"type": "number", "description": "minimum confidence 0.0-1.0"},
			},
		},
	})]
}

use super::{tool_error, tool_result_json, Server};

/// Parse an optional RFC3339 time filter from a query arg. An empty string
/// means "no filter"; a non-empty but unparseable value is a hard error, so a
/// typo'd time-bounded query fails loudly instead of silently returning the
/// full unfiltered result set.
fn parse_time_filter(field: &str, value: &str) -> Result<Option<std::time::SystemTime>, String> {
	if value.is_empty() {
		return Ok(None);
	}
	crate::base::time::parse_rfc3339(value)
		.map(Some)
		.map_err(|()| format!("invalid `{field}` timestamp: {value}"))
}

/// Translate the filter/sort fields of a parsed [`QueryArgs`] into a
/// [`retrieval::score::QueryOptions`]. Returns the tool-error message (`Err`)
/// for an unknown scheme or an unparseable `since`/`before`/`valid_at` filter.
fn build_query_options(p: &QueryArgs) -> Result<retrieval::score::QueryOptions, String> {
	let mut opts = retrieval::score::QueryOptions {
		sort: retrieval::score::SortField::parse(&p.sort),
		ascending: p.ascending,
		source: p.source.clone(),
		kind: p.kind,
		min_conf: p.min_conf,
		since: parse_time_filter("since", &p.since)?,
		before: parse_time_filter("before", &p.before)?,
		valid_at: parse_time_filter("valid_at", &p.valid_at)?,
		..Default::default()
	};
	if let Some(ref s) = p.scheme {
		match crate::base::types::Source::parse_scheme(s) {
			Some(tag) => opts.scheme = Some(tag.to_string()),
			None => return Err(format!("unknown source scheme: {s}")),
		}
	}
	Ok(opts)
}

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
	#[allow(clippy::field_reassign_with_default)]
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

		let mode = retrieval::seed::Mode::parse(&p.mode);
		let answer_on = p.answer;
		let rcfg = &self.cfg.retrieval;

		// Only the answer path is worth caching — it fires HyDE + synthesis (tens
		// of seconds); pure vector retrieval is already sub-millisecond. And only
		// unfiltered, default-sorted queries are cacheable, because a filter or a
		// non-default sort changes the result set/order while the query vector
		// stays the same. The `tag` (mode) keeps the three retrieval modes from
		// colliding on one entry.
		let cacheable = query_is_cacheable(answer_on, rcfg.query_cache_cap, &p);
		let tag = mode as u64;
		let text_hash = retrieval::cache::hash_text(&p.text);

		// Exact-text fast path: a verbatim re-ask (same text, same mode, graph
		// unchanged) returns the cached result WITHOUT even embedding the query —
		// skipping the embedding round-trip on top of the LLM pipeline. `vec` stays
		// `None` on this path; cold-tier fill below is guarded on it.
		let text_hit = if cacheable {
			let g = crate::base::locks::read_recovered(&self.graph);
			self
				.cache
				.lock()
				.ok()
				.and_then(|mut c| c.lookup_text(&g, text_hash, tag))
		} else {
			None
		};

		let (result, vec): (_, Option<Vec<f64>>) = if let Some(hit) = text_hit {
			(hit, None)
		} else {
			let vec = match crate::llm::block_on_in_place(llm.embed(&p.text)) {
				Some(Ok(v)) => v,
				Some(Err(e)) => return tool_error(&format!("embed failed: {e}")),
				None => return tool_error("no tokio runtime"),
			};

			let complete = llm.complete_func();
			let llm_fn: LlmFunc = Arc::new(complete);
			let llm_embed = llm.clone();
			let embed_fn: EmbedFunc =
				Arc::new(
					move |s: &str| match crate::llm::block_on_in_place(llm_embed.embed(s)) {
						Some(r) => r.map_err(|e| e.to_string()),
						None => Err("no tokio runtime".to_string()),
					},
				);

			let opts = match build_query_options(&p) {
				Ok(o) => o,
				Err(e) => return tool_error(&e),
			};

			let (llm_arg, embed_arg) = answer_llm_args(answer_on, &llm_fn, &embed_fn);

			// Semantic lookup: a paraphrase-close prior query (brief read lock for
			// the epoch + cosine scan). On a miss, `query_locked` runs retrieval
			// under its own short-lived lock and does HyDE/rerank/answer with the
			// lock RELEASED — so a slow cloud LLM never pins the read lock long
			// enough to starve writers and trip the 30s watchdog.
			let cached = if cacheable {
				let g = crate::base::locks::read_recovered(&self.graph);
				self
					.cache
					.lock()
					.ok()
					.and_then(|mut c| c.lookup(&g, &vec, tag))
			} else {
				None
			};
			let result = match cached {
				Some(hit) => hit,
				None => {
					let (fresh, epoch) = retrieval::answer::query_locked(
						&self.graph,
						rcfg,
						&vec,
						&p.text,
						mode,
						llm_arg,
						embed_arg,
						Some(opts),
					);
					if cacheable {
						// Stamp with the epoch captured at retrieval time (returned by
						// query_locked), not the live epoch — a write during the LLM
						// phase then correctly invalidates this entry on the next lookup.
						if let Ok(mut c) = self.cache.lock() {
							c.insert(epoch, text_hash, vec.clone(), tag, fresh.clone());
						}
					}
					fresh
				}
			};
			(result, Some(vec))
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
		// Cold-tier fill needs the query vector. On the exact-text fast path we
		// skipped embedding (`vec` is `None`), so cold-tier is skipped too — the
		// cached hot result already reflects what a verbatim re-ask returned.
		if let Some(ref vec) = vec {
			if scored.len() < k {
				// Clone the store handle out from under a brief read guard, then drop
				// the guard before the cold scan.
				let store = crate::base::locks::read_recovered(&self.graph).store();
				let have: std::collections::HashSet<String> =
					scored.iter().map(|s| s.entity.id.clone()).collect();
				if let Some(store) = &store {
					for (entity, score) in store.cold_search(vec, k).unwrap_or_default() {
						if scored.len() >= k {
							break;
						}
						if !have.contains(&entity.id) {
							cold_ids.insert(entity.id.clone());
							scored.push(retrieval::expand::ScoredEntity { entity, score });
						}
					}
				}
			}
		}

		let entities: Vec<serde_json::Value> = {
			let g = match self.graph.read() {
				Ok(g) => g,
				Err(_) => return tool_error("graph lock poisoned"),
			};
			scored
				.iter()
				.take(k)
				.map(|st| {
					// Echo kind/scheme/status directly from the matched
					// Entity so kern_rpc::query can build EntityRef without
					// a second graph lookup. `kind` is the lower-case label
					// (matches `EntityKindLite` serde repr), `scheme` is the
					// stable `Source` URI tag, `status` is `"active"` or
					// `"superseded"` mirroring `EntityStatusLite`.
					// Collect enriched edges so callers can see the specific
					// logical connections between this entity and its neighbours.
					// Only include reasons that have been enriched (have text) to
					// avoid surfacing empty or label-only placeholders.
					let edges: Vec<serde_json::Value> = g
						.kern_of_entity(&st.entity.id)
						.and_then(|kid| g.kerns.get(kid))
						.map(|kern| {
							crate::base::reason::collect_reason_ids(kern, &st.entity.id)
								.into_iter()
								.filter_map(|rid| kern.reasons.get(&rid))
								.filter(|r| r.is_enriched())
								.map(|r| {
									serde_json::json!({
										"from": r.from,
										"to": r.to,
										"kind": r.kind as i32,
										"text": truncate(&r.text, 120),
										"score": r.score,
									})
								})
								.collect()
						})
						.unwrap_or_default();
					let mut v = base_entity_json(&st.entity, st.score);
					v["cold"] = serde_json::Value::Bool(cold_ids.contains(&st.entity.id));
					if !edges.is_empty() {
						v["edges"] = serde_json::Value::Array(edges);
					}
					v
				})
				.collect()
		};

		let mut out = serde_json::json!({"entities": entities});
		if !answer_str.is_empty() {
			out["answer"] = serde_json::Value::String(answer_str);
		}
		tool_result_json(&out)
	}
}

/// Select the optional LLM / embedder handles passed into
/// [`retrieval::answer::query`] for a `query` tool call.
///
/// HyDE expansion, LLM rerank, and answer synthesis are all driven by the
/// `llm` handle and only matter when the caller requested a synthesized
/// `answer`. Gating them here keeps `answer:false` a fast pure-vector
/// retrieval instead of firing several gemma-class generations per query.
fn answer_llm_args<'a>(
	answer: bool,
	llm: &'a LlmFunc,
	embed: &'a EmbedFunc,
) -> (Option<&'a LlmFunc>, Option<&'a EmbedFunc>) {
	if answer {
		(Some(llm), Some(embed))
	} else {
		(None, None)
	}
}

fn entity_detail(
	thought: &crate::base::types::Entity,
	kern_id: &str,
	g: &crate::base::graph::GraphGnn,
) -> serde_json::Value {
	let mut edges = Vec::new();
	if let Some(kern) = g.kerns.get(kern_id) {
		let rids = crate::base::reason::collect_reason_ids(kern, &thought.id);
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

/// The core per-hit JSON shape emitted into `tool_query`'s `entities` array
/// (id/score/conf/text/kind/scheme/status). `tool_query` overlays `cold` and any
/// enriched `edges` on top. Defined ONCE so the kern_rpc-consumed contract has a
/// single source of truth — the envelope tests build on this same fn instead of a
/// hand-mirrored copy that could silently drift.
pub(super) fn base_entity_json(
	entity: &crate::base::types::Entity,
	score: f64,
) -> serde_json::Value {
	let status_str = if entity.is_superseded() {
		"superseded"
	} else {
		"active"
	};
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

/// Whether a `query` call is eligible for the semantic answer cache: only
/// answer-on, cache-enabled, UNFILTERED, default-sorted queries qualify. Any
/// filter or non-default sort changes the result set/order for the same query
/// vector, so caching it would risk serving the wrong results for a later call.
fn query_is_cacheable(answer: bool, cache_cap: usize, p: &QueryArgs) -> bool {
	answer
		&& cache_cap > 0
		&& p.kind.is_none()
		&& p.scheme.is_none()
		&& p.source.is_empty()
		&& p.since.is_empty()
		&& p.before.is_empty()
		&& p.valid_at.is_empty()
		&& p.min_conf == 0.0
		&& p.sort.is_empty()
		&& !p.ascending
}

#[cfg(test)]
mod answer_gating_tests {
	//! The `query` tool must not spend LLM calls (HyDE / rerank / answer
	//! synthesis) unless `answer:true` was requested. Regression guard for
	//! the unconditional-LLM bug that overran the MCP client timeout and
	//! surfaced as `-32000 Connection closed`.
	use super::answer_llm_args;
	use crate::types::{EmbedFunc, LlmFunc};
	use std::sync::Arc;

	#[test]
	fn answer_false_passes_no_llm_or_embedder() {
		let llm: LlmFunc = Arc::new(|_: &str| String::new());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(Vec::new()));
		let (l, e) = answer_llm_args(false, &llm, &embed);
		assert!(l.is_none(), "answer:false must not pass an LLM");
		assert!(e.is_none(), "answer:false must not pass an embedder");
	}

	#[test]
	fn answer_true_passes_llm_and_embedder() {
		let llm: LlmFunc = Arc::new(|_: &str| String::new());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(Vec::new()));
		let (l, e) = answer_llm_args(true, &llm, &embed);
		assert!(l.is_some(), "answer:true must pass an LLM");
		assert!(e.is_some(), "answer:true must pass an embedder");
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
	use super::base_entity_json as build_entity_json;
	use crate::base::types::{ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source};

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

#[cfg(test)]
mod time_filter_tests {
	use super::parse_time_filter;

	#[test]
	fn empty_is_no_filter() {
		assert_eq!(parse_time_filter("since", "").unwrap(), None);
	}

	#[test]
	fn valid_parses_to_some() {
		assert!(parse_time_filter("before", "2026-06-05T09:00:00Z")
			.unwrap()
			.is_some());
	}

	#[test]
	fn nonempty_malformed_is_hard_error() {
		// Full-length but non-numeric year -> hard error naming the field, not a
		// silent unfiltered query.
		let e = parse_time_filter("valid_at", "20XX-06-05T09:00:00Z").unwrap_err();
		assert!(e.contains("valid_at"), "error names the field: {e}");
	}
}

#[cfg(test)]
mod cacheable_tests {
	use super::{query_is_cacheable, QueryArgs};
	use crate::base::types::EntityKind;

	#[test]
	fn plain_answer_query_is_cacheable() {
		let p = QueryArgs::default();
		assert!(
			query_is_cacheable(true, 256, &p),
			"unfiltered, default-sorted answer query caches"
		);
	}

	#[test]
	fn answer_off_or_disabled_cache_is_not_cacheable() {
		let p = QueryArgs::default();
		assert!(
			!query_is_cacheable(false, 256, &p),
			"answer:false is never cached"
		);
		assert!(
			!query_is_cacheable(true, 0, &p),
			"cache_cap 0 disables caching"
		);
	}

	#[test]
	fn any_filter_or_nondefault_sort_disables_caching() {
		// Each mutation alone must flip cacheable to false: a filter changes the
		// result set and a sort changes the order, both for the same query vector.
		type Mutator = fn(&mut QueryArgs);
		let cases: Vec<(&str, Mutator)> = vec![
			("source", |p| p.source = "github".into()),
			("kind", |p| p.kind = Some(EntityKind::Fact)),
			("scheme", |p| p.scheme = Some("file".into())),
			("since", |p| p.since = "2026-01-01T00:00:00Z".into()),
			("before", |p| p.before = "2026-01-01T00:00:00Z".into()),
			("valid_at", |p| p.valid_at = "2026-01-01T00:00:00Z".into()),
			("min_conf", |p| p.min_conf = 0.5),
			("sort", |p| p.sort = "date".into()),
			("ascending", |p| p.ascending = true),
		];
		for (name, mutate) in cases {
			let mut p = QueryArgs::default();
			mutate(&mut p);
			assert!(
				!query_is_cacheable(true, 256, &p),
				"`{name}` set must disable caching"
			);
		}
	}
}
