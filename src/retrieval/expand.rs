use crate::base::graph::GraphGnn;
use crate::base::math::cosine;
use crate::base::reason::collect_reason_ids;
use crate::base::search::EntityHit;
use crate::base::types::*;
use crate::config::RetrievalConfig;
use crate::retrieval::heap::{BeamHeap, HeapItem};
use crate::retrieval::seed::Weights;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct PathChain {
	pub nodes: Vec<String>,
	pub score: f64,
}

#[derive(Debug, Clone)]
pub struct ScoredEntity {
	pub entity: Entity,
	pub score: f64,
}

pub struct ExpandResult {
	pub scored: Vec<ScoredEntity>,
	pub chains: Vec<PathChain>,
}

pub fn expand(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f64],
	seeds: &[EntityHit],
	w: Weights,
) -> ExpandResult {
	let mut heap = BeamHeap::new();
	let mut visited = HashSet::new();
	let mut results: HashMap<String, f64> = HashMap::new();
	let mut chains: Vec<PathChain> = Vec::new();
	let mut global_best: f64 = 0.0;

	for s in seeds {
		heap.push(HeapItem {
			entity_id: s.entity_id.clone(),
			score: s.score,
			chain: vec![s.entity_id.clone()],
		});
	}

	let max_expansions = cfg.max_expansions;
	let decay = cfg.decay;
	let refine_tw = cfg.refine_traversal_weight;
	let refine_cap = cfg.refine_boost_cap;
	let mut expansions = 0;

	while let Some(item) = heap.pop() {
		if expansions >= max_expansions {
			break;
		}
		expansions += 1;

		if !visited.insert(item.entity_id.clone()) {
			continue;
		}

		let entry = results.entry(item.entity_id.clone()).or_insert(0.0);
		if item.score > *entry {
			*entry = item.score;
		}

		if item.score > global_best {
			global_best = item.score;
		}
		let threshold = global_best * decay;

		if item.chain.len() > 1 {
			chains.push(PathChain {
				nodes: item.chain.clone(),
				score: item.score,
			});
		}

		let (_thought, kern) = match find_entity_and_kern(g, &item.entity_id) {
			Some(r) => r,
			None => continue,
		};

		let reason_ids = collect_reason_ids(kern, &item.entity_id);

		for rid in &reason_ids {
			let reason = match kern.reasons.get(rid) {
				Some(r) => r,
				None => continue,
			};

			if reason.is_remote() {
				continue;
			}

			if reason.kind == ReasonKind::Spawn && !reason.to.is_empty() {
				continue;
			}

			let neighbor_id = if reason.from == item.entity_id {
				&reason.to
			} else {
				&reason.from
			};

			if neighbor_id.is_empty() || visited.contains(neighbor_id) {
				continue;
			}

			let neighbor = match find_entity_in_graph(g, neighbor_id) {
				Some(t) => t,
				None => continue,
			};

			let score = score_neighbor(query_vec, &neighbor, reason, w, refine_tw, refine_cap);
			if score < threshold {
				continue;
			}

			let mut chain = item.chain.clone();
			chain.push(rid.clone());
			chain.push(neighbor_id.clone());

			heap.push(HeapItem {
				entity_id: neighbor_id.clone(),
				score,
				chain,
			});
		}
	}

	let scored: Vec<ScoredEntity> = results
		.into_iter()
		.filter_map(|(id, score)| {
			find_entity_in_graph(g, &id).map(|t| ScoredEntity { entity: t, score })
		})
		.collect();

	ExpandResult { scored, chains }
}

pub fn score_neighbor(
	query_vec: &[f64],
	neighbor: &Entity,
	reason: &Reason,
	w: Weights,
	refine_traversal_weight: f64,
	refine_boost_cap: f64,
) -> f64 {
	let content_score = if neighbor.has_vector() {
		cosine(query_vec, &neighbor.vector)
	} else {
		0.0
	};
	let reason_score = if reason.has_vector() {
		cosine(query_vec, &reason.vector)
	} else {
		0.0
	};
	let traversal_boost = ((reason.traversal_count.value() as f64 + 1.0).ln()
		* refine_traversal_weight)
		.min(refine_boost_cap);
	let edge_score = (reason.score.clamp(0.0, 1.0) + traversal_boost).min(1.0);

	w.content * content_score + w.reason * reason_score + w.edge * edge_score
}


fn find_entity_and_kern<'a>(g: &'a GraphGnn, id: &str) -> Option<(&'a Entity, &'a Kern)> {
	if let Some(kid) = g.kern_of_entity(id) {
		if let Some(kern) = g.loaded(kid) {
			if let Some(t) = kern.entities.get(id) {
				return Some((t, kern));
			}
		}
	}
	for kern in g.all() {
		if let Some(t) = kern.entities.get(id) {
			return Some((t, kern));
		}
	}
	None
}

pub fn find_entity_in_graph(g: &GraphGnn, id: &str) -> Option<Entity> {
	find_entity_and_kern(g, id).map(|(t, _)| t.clone())
}

