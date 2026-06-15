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

/// Resolve an entity and its owning kern, returning both by reference (no clone
/// on the hot retrieval path). Two-pass by design: first the O(1) hot path via
/// the `kern_of_entity` index when the entity lives in a loaded kern; then a full
/// scan over all loaded kerns as a fallback for orphans whose index entry is
/// missing or stale (e.g. an entity migrated between kerns before the index
/// caught up). `None` only if the id is absent from every loaded kern.
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;

	fn ent(id: &str, vector: Vec<f64>) -> Entity {
		Entity {
			id: id.into(),
			vector,
			..Default::default()
		}
	}
	fn edge(from: &str, to: &str, score: f64) -> Reason {
		Reason {
			id: format!("{from}->{to}"),
			from: from.into(),
			to: to.into(),
			score,
			kind: ReasonKind::Similarity,
			..Default::default()
		}
	}

	#[test]
	fn score_neighbor_pure_content_weight_is_cosine() {
		let neighbor = ent("n", vec![1.0, 0.0]);
		let r = edge("a", "n", 0.5); // no reason vector -> reason component 0
		let w = Weights {
			content: 1.0,
			reason: 0.0,
			edge: 0.0,
			lexical: 0.0,
		};
		let s = score_neighbor(&[1.0, 0.0], &neighbor, &r, w, 0.1, 0.3);
		assert!(
			(s - 1.0).abs() < 1e-9,
			"query aligned with neighbour -> 1.0"
		);
	}

	#[test]
	fn score_neighbor_pure_edge_weight_uses_clamped_reason_score() {
		let neighbor = ent("n", vec![]); // no vector -> content 0
		let r = edge("a", "n", 0.4); // traversal_count 0 -> ln(1)*tw = 0 boost
		let w = Weights {
			content: 0.0,
			reason: 0.0,
			edge: 1.0,
			lexical: 0.0,
		};
		let s = score_neighbor(&[1.0, 0.0], &neighbor, &r, w, 0.1, 0.3);
		assert!(
			(s - 0.4).abs() < 1e-9,
			"edge component is the clamped reason score"
		);
	}

	#[test]
	fn expand_walks_edges_from_seed_and_records_a_chain() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for id in ["a", "b", "c"] {
			k.entities.insert(id.into(), ent(id, vec![1.0, 0.0]));
		}
		add_reason(&mut k, edge("a", "b", 0.9));
		add_reason(&mut k, edge("b", "c", 0.9));
		g.kerns.insert("kx".into(), k);

		let cfg = RetrievalConfig::default();
		let seeds = [EntityHit {
			entity_id: "a".into(),
			score: 1.0,
		}];
		let w = Weights {
			content: 1.0,
			reason: 0.0,
			edge: 0.0,
			lexical: 0.0,
		};
		let res = expand(&g, &cfg, &[1.0, 0.0], &seeds, w);

		let ids: HashSet<&str> = res.scored.iter().map(|s| s.entity.id.as_str()).collect();
		assert!(ids.contains("a"), "the seed is scored");
		assert!(
			ids.contains("b"),
			"the 1-hop neighbour is reached via the edge"
		);
		assert!(
			res.chains.iter().any(|c| c.nodes.len() >= 3),
			"a multi-hop chain (entity, reason, entity) is recorded"
		);
	}
}
