use std::sync::{Arc, RwLock};

use crate::base::constants::{
	DEFAULT_SEED_K, KERN_INNER_RADIUS, KERN_OUTER_RADIUS, PROVENANCE_SCORE,
	QUESTION_RESOLVE_THRESHOLD,
};
use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::math::reason_id;
use crate::base::reason::add_reason;
use crate::base::search::search_all_unlocked;
use crate::base::types::{Reason, ReasonKind};
use crate::base::util;
use crate::config::TickConfig;

use super::cluster::{
	anchor_prompt, centroid_thought, largest_cohesive_cluster_for_naming, vector_cluster,
};
use super::queue::{task, task_extra, Queue, TaskKind};

pub use crate::types::{EmbedFunc, LlmFunc};
pub type BroadcastQuestionFunc = Arc<dyn Fn(&str, &str, &[f64], &str) + Send + Sync>;

/// Strip a leading `Theme:`/`Name:`/`Label:` label (a few case variants) that the
/// naming LLM sometimes prepends, returning the trimmed remainder. Only the first
/// matching prefix is removed. Pure, so the parsing is unit-testable apart from
/// `do_name`'s graph/LLM plumbing.
fn strip_name_prefixes(raw: &str) -> String {
	let mut name = raw.trim().to_string();
	for pfx in &["Theme:", "Name:", "Label:", "theme:", "name:"] {
		if let Some(after) = name.strip_prefix(pfx) {
			name = after.trim().to_string();
			break;
		}
	}
	name
}

/// Seed up to 3 dangling Question edges for a freshly ingested entity.
///
/// Relocated from the ingest worker's `place_chunks` (where it was one
/// BLOCKING reason-LLM call per placed chunk on the commit path — measured
/// live: a one-line sync ingest queued 69.7 minutes behind LLM-bound jobs).
/// The ingest worker now enqueues a `SeedQuestions` tick task per placed
/// chunk via its defer hook, and the tick — which already serializes LLM
/// work off the interactive path — runs this. Locks: text + root id read
/// under one read guard, the LLM call runs UNLOCKED, edges written under one
/// write guard (same discipline as `do_name`).
pub fn do_seed_questions(g: &Arc<RwLock<GraphGnn>>, entity_id: &str, llm: Option<&LlmFunc>) {
	let Some(llm) = llm else { return };
	let (text, root_id) = {
		let g = read_recovered(g);
		let Some(kid) = g.kern_of_entity(entity_id).map(|s| s.to_string()) else {
			return;
		};
		let Some(text) = g
			.kerns
			.get(&kid)
			.and_then(|k| k.entities.get(entity_id))
			.map(|e| e.text())
		else {
			return;
		};
		(text, g.root.id.clone())
	};
	if text.trim().is_empty() {
		return;
	}

	let prompt = format!(
		"Given this knowledge chunk, generate up to 3 questions that this chunk answers. \
		 One question per line. No numbering.\n\n{text}"
	);
	let response = llm(&prompt);
	if response.is_empty() {
		return;
	}
	let questions: Vec<String> = response
		.lines()
		.map(|l| l.trim().to_string())
		.filter(|l| !l.is_empty())
		.take(3)
		.collect();

	let mut g = write_recovered(g);
	for q in questions {
		let rid = reason_id(entity_id, "", ReasonKind::Question, &q, "");
		let reason = Reason {
			id: rid,
			from: entity_id.to_string(),
			to: String::new(),
			to_kern_id: String::new(),
			to_net_id: String::new(),
			kind: ReasonKind::Question,
			dirty: false,
			text: q,
			vector: Vec::new(),
			score: 0.5,
			traversal_count: crate::crdt::GCounter::new(),
			producer_id: String::new(),
		};
		if let Some(kern) = g.kerns.get_mut(&root_id) {
			add_reason(kern, reason);
		}
	}
}

/// Under a read lock, decide how to name an unnamed kern: cluster its entities,
/// pick the largest cohesive cluster, and return its anchor `(prompt, centroid
/// id, parent id)`. `None` when the kern is gone, already named, or has no
/// cohesive cluster worth naming.
fn naming_prompt(
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	cfg: &TickConfig,
) -> Option<(String, Option<String>, String)> {
	let graph = read_recovered(g);
	let kern = graph.loaded(kern_id)?;
	if kern.is_named() {
		return None;
	}
	let entities: Vec<_> = kern.entities.values().collect();
	let clusters = vector_cluster(&entities, cfg.max_cluster_sample);
	let idx = largest_cohesive_cluster_for_naming(&clusters)?;
	let prompt = anchor_prompt(&clusters[idx]);
	let centroid_id = centroid_thought(&clusters[idx]).map(|t| t.id.clone());
	let parent_id = kern.parent.clone();
	Some((prompt, centroid_id, parent_id))
}

pub fn do_name(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	cfg: &TickConfig,
	llm: Option<&LlmFunc>,
	embed: Option<&EmbedFunc>,
) {
	let llm = match llm {
		Some(f) => f,
		None => return,
	};

	let (prompt, centroid_id, parent_id) = match naming_prompt(g, kern_id, cfg) {
		Some(t) => t,
		None => return,
	};

	let raw = llm(&prompt);
	let name_text = strip_name_prefixes(&raw);
	if name_text.is_empty() {
		return;
	}
	let name_vec = embed.and_then(|e| e(&name_text).ok());

	let promoted_to_root = {
		let mut graph = write_recovered(g);
		let kern = match graph.kerns.get_mut(kern_id) {
			Some(k) => k,
			None => return,
		};
		if kern.is_named() {
			return;
		}
		kern.anchor_text = name_text.clone();
		kern.anchor_vec = name_vec.unwrap_or_default();
		kern.inner_radius = KERN_INNER_RADIUS;
		kern.outer_radius = KERN_OUTER_RADIUS;

		if let Some(ref cid) = centroid_id {
			let mut spawn = Reason {
				kind: ReasonKind::Spawn,
				from: cid.clone(),
				to_kern_id: kern_id.to_string(),
				score: PROVENANCE_SCORE,
				..Default::default()
			};
			spawn.id = reason_id(&spawn.from, "", spawn.kind, &spawn.to_kern_id, "");
			kern.spawn_reason_id = spawn.id.clone();
			if let Some(parent) = graph.kerns.get_mut(&parent_id) {
				add_reason(parent, spawn);
			}
		}

		// Emergent promotion: a dense cluster that crystallized inside the
		// `generic` catch-all becomes a first-class anchor directly under the
		// root, so future matching memories route to it instead of generic.
		crate::base::accept::promote_to_root_if_generic(&mut graph, kern_id)
	};

	{
		let graph = read_recovered(g);
		if let Some(kern) = graph.loaded(kern_id) {
			for r in kern.reasons.values() {
				if r.is_enriched() || r.kind == ReasonKind::Spawn || r.kind == ReasonKind::Question {
					continue;
				}
				q.enqueue(task_extra(TaskKind::Enrich, kern_id, &r.id));
			}
		}
	}
	q.enqueue(task(TaskKind::Persist, kern_id));
	if !parent_id.is_empty() {
		q.enqueue(task(TaskKind::Persist, &parent_id));
	}
	// Promotion rewired the root's children — persist it too.
	if promoted_to_root {
		let root_id = read_recovered(g).root.id.clone();
		q.enqueue(task(TaskKind::Persist, &root_id));
	}
}

pub fn do_enrich(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	rid: &str,
	llm: Option<&LlmFunc>,
	embed: Option<&EmbedFunc>,
) {
	let (llm, embed) = match (llm, embed) {
		(Some(l), Some(e)) => (l, e),
		_ => return,
	};

	let prompt = {
		let graph = read_recovered(g);
		let kern = match graph.loaded(kern_id) {
			Some(k) => k,
			None => return,
		};
		let r = match kern.reasons.get(rid) {
			Some(r) => r,
			None => return,
		};
		if r.is_enriched() || r.kind == ReasonKind::Spawn || r.kind == ReasonKind::Question {
			return;
		}
		let from = match kern.entities.get(&r.from) {
			Some(t) => t,
			None => return,
		};
		let to = match kern.entities.get(&r.to) {
			Some(t) => t,
			None => return,
		};
		util::explain_relationship_prompt(&from.text(), &to.text())
	};

	let text = llm(&prompt);
	if text.is_empty() {
		return;
	}
	let text = text.trim().to_string();
	let vec = embed(&text).ok();

	{
		let mut graph = write_recovered(g);
		let mut new_vec: Option<(String, Vec<f64>)> = None;
		if let Some(kern) = graph.kerns.get_mut(kern_id) {
			if let Some(r) = kern.reasons.get_mut(rid) {
				if !r.is_enriched() {
					r.text = text;
					if let Some(v) = vec {
						r.vector = v.clone();
						new_vec = Some((rid.to_string(), v));
					}
				}
			}
		}
		if let Some((rid, v)) = new_vec {
			graph.reason_idx.delete(&rid);
			graph.reason_idx.insert(rid, v);
		}
	}

	q.enqueue(task(TaskKind::Persist, kern_id));
	q.enqueue(task(TaskKind::GnnPropagate, kern_id));
}

pub fn do_resolve(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	rid: &str,
	bq: Option<&BroadcastQuestionFunc>,
) {
	// Phase 1 (read guard): snapshot the question vector and run the read-only
	// whole-graph ANN search. The search is the expensive part — holding only a
	// read guard here lets other ticks read/write concurrently instead of
	// serializing every daemon operation behind one resolve.
	let top_hit = {
		let graph = read_recovered(g);
		let kern = match graph.loaded(kern_id) {
			Some(k) => k,
			None => return,
		};
		let r = match kern.reasons.get(rid) {
			Some(r) => r,
			None => return,
		};
		if r.kind != ReasonKind::Question || !r.to.is_empty() {
			return;
		}
		let vec = r.vector.clone();
		search_all_unlocked(&graph, &vec, DEFAULT_SEED_K)
			.into_iter()
			.next()
			.filter(|h| h.score >= QUESTION_RESOLVE_THRESHOLD)
			.map(|h| h.entity_id)
	};

	// Phase 2a (write guard, mutation only): resolved locally. The read guard
	// was dropped, so re-validate under the write guard — another tick could
	// have resolved or removed this question in between.
	if let Some(entity_id) = top_hit {
		{
			let mut graph = write_recovered(g);
			let kern = match graph.kerns.get_mut(kern_id) {
				Some(k) => k,
				None => return,
			};
			let r = match kern.reasons.get_mut(rid) {
				Some(r) => r,
				None => return,
			};
			if r.kind != ReasonKind::Question || !r.to.is_empty() {
				return;
			}
			r.to = entity_id;
			r.kind = ReasonKind::Similarity;
		}
		q.enqueue(task(TaskKind::Persist, kern_id));
		return;
	}

	// Phase 2b (read guard): unresolved locally — snapshot the question and
	// broadcast it to peers. Read-only, so no write guard needed.
	let broadcast_data = if bq.is_some() {
		let graph = read_recovered(g);
		graph.loaded(kern_id).and_then(|kern| {
			kern.reasons.get(rid).map(|r| {
				(
					r.id.clone(),
					r.from.clone(),
					r.vector.clone(),
					r.text.clone(),
				)
			})
		})
	} else {
		None
	};

	if let (Some(bq), Some((id, from_id, rvec, rtext))) = (bq, broadcast_data) {
		bq(&id, &from_id, &rvec, &rtext);
	}
}

/// Fold the disk-backed entity index's in-RAM delta into a fresh DiskANN snapshot
/// and reset it (see [`GraphGnn::consolidate_disk_index`]). Graph-global — the
/// task carries no kern. No-op when the entity index is not disk-backed.
pub fn do_disk_consolidate(g: &Arc<RwLock<GraphGnn>>) {
	write_recovered(g).consolidate_disk_index();
}

pub fn do_persist(g: &Arc<RwLock<GraphGnn>>, kern_id: &str) {
	let graph = read_recovered(g);
	let store = match graph.store() {
		Some(s) => s,
		None => return,
	};
	// The root carries authoritative fields (purpose/descriptors/radii) that
	// live on `graph.root`, not the map entry — persist it through the same
	// merge `save_all` uses so a root Persist task can't drop them.
	if kern_id == graph.root.id {
		let _ = store.save_one_kern(&crate::base::persist::merged_root(&graph));
		return;
	}
	let kern = match graph.loaded(kern_id) {
		Some(k) => k,
		None => return,
	};
	let _ = store.save_one_kern(kern);
}

/// Re-embed every dirty entity (and recompute dirty reason vectors) in `kern_id`,
/// then clear the flag and rebuild the index. The dirty flag is the durable
/// source of truth — set on edit, cleared here once the stale vector is replaced.
pub fn do_reembed(g: &Arc<RwLock<GraphGnn>>, kern_id: &str, embed: Option<&EmbedFunc>) {
	let Some(embed) = embed else { return };

	// Snapshot dirty entity (id, text) under a read guard.
	let dirty_ents: Vec<(String, String)> = {
		let g = read_recovered(g);
		let Some(k) = g.kerns.get(kern_id) else {
			return;
		};
		k.entities
			.values()
			.filter(|e| e.dirty)
			.map(|e| (e.id.clone(), e.text()))
			.collect()
	};

	// Embed outside the lock (network I/O).
	let mut new_vecs: Vec<(String, Vec<f64>)> = Vec::new();
	for (id, text) in &dirty_ents {
		if let Ok(v) = embed(text) {
			if !v.is_empty() {
				new_vecs.push((id.clone(), v));
			}
		}
	}

	// Are there dirty reasons to recompute too?
	let has_dirty_reasons = {
		let g = read_recovered(g);
		g.kerns
			.get(kern_id)
			.map(|k| k.reasons.values().any(|r| r.dirty))
			.unwrap_or(false)
	};

	if new_vecs.is_empty() && !has_dirty_reasons {
		return;
	}

	// Write back under a write guard.
	{
		let mut g = write_recovered(g);
		let Some(k) = g.kerns.get_mut(kern_id) else {
			return;
		};
		for (id, v) in &new_vecs {
			if let Some(e) = k.entities.get_mut(id) {
				e.vector = v.clone();
				e.gnn_vector = v.clone();
				e.dirty = false;
			}
		}
		// Recompute dirty reason vectors as the mean of their (now-updated)
		// endpoint vectors; clear the flag.
		let endpoint = |k: &crate::base::types::Kern, id: &str| -> Option<Vec<f64>> {
			k.entities
				.get(id)
				.map(|e| e.vector.clone())
				.filter(|v| !v.is_empty())
		};
		let reason_ids: Vec<String> = k
			.reasons
			.values()
			.filter(|r| r.dirty)
			.map(|r| r.id.clone())
			.collect();
		for rid in reason_ids {
			let (from, to) = match k.reasons.get(&rid) {
				Some(r) => (r.from.clone(), r.to.clone()),
				None => continue,
			};
			let nv = match (endpoint(k, &from), endpoint(k, &to)) {
				(Some(fv), Some(tv)) => Some(crate::base::math::average_vec(&fv, &tv)),
				_ => None,
			};
			if let Some(r) = k.reasons.get_mut(&rid) {
				// Recomputed the edge vector — correction recorded, clear dirty. When
				// an endpoint isn't embedded yet (cold/unembedded) `nv` is None: leave
				// the edge dirty so a later sweep retries once both endpoints have
				// vectors, rather than pinning a stale vector.
				if let Some(v) = nv {
					r.vector = v;
					r.dirty = false;
				}
			}
		}
		g.rebuild_index();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{Entity, Kern};
	use std::sync::{Arc, RwLock};

	#[test]
	fn do_seed_questions_adds_question_edges_for_the_entity() {
		// Question seeding moved OFF the ingest commit path (it was one blocking
		// reason-LLM call per placed chunk inside the worker — measured live: a
		// one-line sync ingest queued 69.7 minutes behind LLM-bound jobs). The
		// tick owns it now: read the entity text, ask the LLM, attach dangling
		// Question edges to the root kern — same shape the resolver consumes.
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let mut e = Entity {
			id: "e1".into(),
			..Default::default()
		};
		e.set_text("the spawn gate shipped today".into());
		g.kerns
			.get_mut(&root)
			.unwrap()
			.entities
			.insert("e1".into(), e);
		// Repopulate entity_kern (private) from the kern maps, like load does.
		g.rebuild_index();
		let g = Arc::new(RwLock::new(g));

		let llm: LlmFunc =
			Arc::new(|_p: &str| "What shipped today?\nWhen did the gate ship?".to_string());
		do_seed_questions(&g, "e1", Some(&llm));

		let gg = g.read().unwrap();
		let qs: Vec<_> = gg
			.kerns
			.get(&root)
			.unwrap()
			.reasons
			.values()
			.filter(|r| r.kind == ReasonKind::Question && r.from == "e1" && r.to.is_empty())
			.collect();
		assert_eq!(qs.len(), 2, "one dangling Question edge per LLM line");
	}

	#[test]
	fn do_seed_questions_is_a_noop_without_llm_or_entity() {
		let g = Arc::new(RwLock::new(GraphGnn::new()));
		// No LLM -> noop.
		do_seed_questions(&g, "e1", None);
		// LLM but unknown entity -> noop (no panic, no edges).
		let llm: LlmFunc = Arc::new(|_p: &str| "Q?".to_string());
		do_seed_questions(&g, "missing", Some(&llm));
		let gg = g.read().unwrap();
		let root = gg.root.id.clone();
		assert!(
			gg.kerns.get(&root).unwrap().reasons.is_empty(),
			"no edges minted"
		);
	}

	#[test]
	fn do_reembed_clears_dirty_and_sets_vector() {
		let mut g = GraphGnn::new();
		let kid = "k1".to_string();
		let mut kern = Kern::new(kid.clone(), "");
		let mut e = Entity {
			id: "e1".into(),
			dirty: true,
			..Default::default()
		};
		e.set_text("hello world".into());
		kern.entities.insert(e.id.clone(), e);
		g.kerns.insert(kid.clone(), kern);
		let g = Arc::new(RwLock::new(g));
		let embed: EmbedFunc = Arc::new(|_t: &str| Ok(vec![0.1, 0.2, 0.3]));
		do_reembed(&g, &kid, Some(&embed));
		let g = g.read().unwrap();
		let e = g.kerns.get(&kid).unwrap().entities.get("e1").unwrap();
		assert!(!e.dirty, "dirty must be cleared after reembed");
		assert_eq!(e.vector, vec![0.1, 0.2, 0.3]);
	}

	#[test]
	fn do_reembed_recomputes_dirty_reason_as_endpoint_mean() {
		let mut g = GraphGnn::new();
		let kid = "k1".to_string();
		let mut kern = Kern::new(kid.clone(), "");
		// Two already-embedded (non-dirty) entities and one dirty edge between them.
		kern.entities.insert(
			"a".into(),
			Entity {
				id: "a".into(),
				vector: vec![1.0, 0.0],
				..Default::default()
			},
		);
		kern.entities.insert(
			"b".into(),
			Entity {
				id: "b".into(),
				vector: vec![0.0, 1.0],
				..Default::default()
			},
		);
		add_reason(
			&mut kern,
			Reason {
				id: "a->b".into(),
				from: "a".into(),
				to: "b".into(),
				dirty: true,
				..Default::default()
			},
		);
		g.kerns.insert(kid.clone(), kern);
		let g = Arc::new(RwLock::new(g));

		// Embedder is unused here (no dirty entities), but required by the signature.
		let embed: EmbedFunc = Arc::new(|_t: &str| Ok(vec![9.0, 9.0]));
		do_reembed(&g, &kid, Some(&embed));

		let g = g.read().unwrap();
		let r = g.kerns.get(&kid).unwrap().reasons.get("a->b").unwrap();
		assert!(!r.dirty, "dirty reason cleared once recomputed");
		assert_eq!(
			r.vector,
			vec![0.5, 0.5],
			"reason vector is the mean of endpoint vectors"
		);
	}

	#[test]
	fn do_resolve_links_question_to_nearest_entity_above_threshold() {
		// A pending Question whose vector matches an indexed entity should be
		// resolved to that entity (kind flips to Similarity, `to` is filled).
		// Exercises the read-search / write-mutate split: search runs under a
		// read guard, the mutation re-validates under a write guard.
		let mut g = GraphGnn::new();
		let kid = "k1".to_string();
		let mut kern = Kern::new(kid.clone(), "");
		kern.entities.insert(
			"target".into(),
			Entity {
				id: "target".into(),
				vector: vec![1.0, 0.0, 0.0],
				..Default::default()
			},
		);
		kern.entities.insert(
			"asker".into(),
			Entity {
				id: "asker".into(),
				vector: vec![0.0, 1.0, 0.0],
				..Default::default()
			},
		);
		add_reason(
			&mut kern,
			Reason {
				id: "q1".into(),
				from: "asker".into(),
				to: String::new(),
				kind: ReasonKind::Question,
				vector: vec![1.0, 0.0, 0.0], // identical to `target` -> cosine 1.0
				..Default::default()
			},
		);
		g.kerns.insert(kid.clone(), kern);
		g.rebuild_index(); // populate entity_idx so search_all_unlocked can hit
		let g = Arc::new(RwLock::new(g));

		let q = Queue::new(16);
		do_resolve(&q, &g, &kid, "q1", None);

		let g = g.read().unwrap();
		let r = g.kerns.get(&kid).unwrap().reasons.get("q1").unwrap();
		assert_eq!(
			r.kind,
			ReasonKind::Similarity,
			"resolved question becomes a Similarity edge"
		);
		assert_eq!(r.to, "target", "linked to the nearest indexed entity");
	}

	#[test]
	fn do_resolve_ignores_non_question_or_already_linked() {
		// Guard clauses: a non-Question, or a Question already linked, is left
		// untouched (and never takes the write guard).
		let mut g = GraphGnn::new();
		let kid = "k1".to_string();
		let mut kern = Kern::new(kid.clone(), "");
		kern.entities.insert(
			"target".into(),
			Entity {
				id: "target".into(),
				vector: vec![1.0, 0.0],
				..Default::default()
			},
		);
		add_reason(
			&mut kern,
			Reason {
				id: "linked".into(),
				from: "x".into(),
				to: "y".into(), // already linked
				kind: ReasonKind::Question,
				vector: vec![1.0, 0.0],
				..Default::default()
			},
		);
		g.kerns.insert(kid.clone(), kern);
		g.rebuild_index();
		let g = Arc::new(RwLock::new(g));

		let q = Queue::new(16);
		do_resolve(&q, &g, &kid, "linked", None);

		let g = g.read().unwrap();
		let r = g.kerns.get(&kid).unwrap().reasons.get("linked").unwrap();
		assert_eq!(
			r.kind,
			ReasonKind::Question,
			"already-linked question is untouched"
		);
		assert_eq!(r.to, "y", "existing link preserved");
	}

	#[test]
	fn strip_name_prefixes_removes_first_known_label_only() {
		assert_eq!(
			strip_name_prefixes("Theme: rust ownership"),
			"rust ownership"
		);
		assert_eq!(
			strip_name_prefixes("  name:  caching layer  "),
			"caching layer"
		);
		assert_eq!(strip_name_prefixes("Label:x"), "x");
		// No known prefix -> trimmed verbatim.
		assert_eq!(strip_name_prefixes("  plain phrase "), "plain phrase");
		// Only the first prefix is stripped.
		assert_eq!(strip_name_prefixes("Theme: Name: nested"), "Name: nested");
	}
}
