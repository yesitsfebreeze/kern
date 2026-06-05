use std::sync::{Arc, RwLock};

use crate::base::constants::{
	DEFAULT_SEED_K, KERN_INNER_RADIUS, KERN_OUTER_RADIUS, PROVENANCE_SCORE,
	QUESTION_RESOLVE_THRESHOLD,
};
use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::math::reason_id;
use crate::base::persist::save_kern;
use crate::base::reason::add_reason;
use crate::base::search::search_all_unlocked;
use crate::base::types::{Reason, ReasonKind};
use crate::base::util;
use crate::config::TickConfig;

use super::cluster::{
	centroid_thought, largest_cohesive_cluster_for_naming, purpose_prompt, vector_cluster,
};
use super::queue::{task, task_extra, Queue, TaskKind};

pub use crate::types::{EmbedFunc, LlmFunc};
pub type BroadcastQuestionFunc = Arc<dyn Fn(&str, &str, &[f64], &str) + Send + Sync>;

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

	let (prompt, centroid_id, parent_id) = {
		let graph = read_recovered(g);
		let kern = match graph.loaded(kern_id) {
			Some(k) => k,
			None => return,
		};
		if kern.is_named() {
			return;
		}
		let entity_count = kern.entities.len();
		let entities: Vec<_> = kern.entities.values().collect();
		let clusters = vector_cluster(&entities, cfg.max_cluster_sample);
		let idx = match largest_cohesive_cluster_for_naming(&clusters) {
			Some(i) => i,
			None => {
				let _ = entity_count;
				return;
			}
		};
		let prompt = purpose_prompt(&clusters[idx]);
		let centroid_id = centroid_thought(&clusters[idx]).map(|t| t.id.clone());
		let parent_id = kern.parent.clone();
		(prompt, centroid_id, parent_id)
	};

	let raw = llm(&prompt);
	let mut name_text = raw.trim().to_string();
	for pfx in &["Theme:", "Name:", "Label:", "theme:", "name:"] {
		if let Some(after) = name_text.strip_prefix(pfx) {
			name_text = after.trim().to_string();
			break;
		}
	}
	if name_text.is_empty() {
		return;
	}
	let name_vec = embed.and_then(|e| e(&name_text).ok());

	{
		let mut graph = write_recovered(g);
		let kern = match graph.kerns.get_mut(kern_id) {
			Some(k) => k,
			None => return,
		};
		if kern.is_named() {
			return;
		}
		kern.purpose_text = name_text.clone();
		kern.purpose_vec = name_vec.unwrap_or_default();
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
	}

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
let mut graph = write_recovered(g);

	let vec = {
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
		r.vector.clone()
	};

	let hits = search_all_unlocked(&graph, &vec, DEFAULT_SEED_K);
	if !hits.is_empty() && hits[0].score >= QUESTION_RESOLVE_THRESHOLD {
		if let Some(kern) = graph.kerns.get_mut(kern_id) {
			if let Some(r) = kern.reasons.get_mut(rid) {
				r.to = hits[0].entity_id.clone();
				r.kind = ReasonKind::Similarity;
			}
		}
		drop(graph);
		q.enqueue(task(TaskKind::Persist, kern_id));
		return;
	}

	let broadcast_data = if bq.is_some() {
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
	drop(graph);

	if let (Some(bq), Some((id, from_id, rvec, rtext))) = (bq, broadcast_data) {
		bq(&id, &from_id, &rvec, &rtext);
	}
}

pub fn do_persist(g: &Arc<RwLock<GraphGnn>>, kern_id: &str) {
	let graph = read_recovered(g);
	if graph.data_dir.is_empty() {
		return;
	}
	// The root carries authoritative fields (purpose/descriptors/radii) that
	// live on `graph.root`, not the map entry — persist it through the same
	// merge `save_all` uses so a root Persist task can't drop them.
	if kern_id == graph.root.id {
		let _ = save_kern(&graph.data_dir, &crate::base::persist::merged_root(&graph));
		return;
	}
	let kern = match graph.loaded(kern_id) {
		Some(k) => k,
		None => return,
	};
	let _ = save_kern(&graph.data_dir, kern);
}
