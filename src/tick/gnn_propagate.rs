use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::types::Kern;
use crate::gnn::graph::Graph;
use crate::gnn::propagate::{self, GnnConfig, GnnSnapshot};

use super::queue::{task, Queue, TaskKind};

pub fn do_gnn_propagate(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	cfg: &GnnConfig,
) {
	let snap = {
		let graph = read_recovered(g);
		let kern = match graph.loaded(kern_id) {
			Some(k) => k,
			None => return,
		};
		if kern.entities.len() < cfg.min_thoughts {
			return;
		}
		build_gnn_snapshot(kern, cfg)
	};

	let snap = match snap {
		Some(s) if !s.pos_edges.is_empty() => s,
		_ => return,
	};

	if let Ok(res) = propagate::run_learned_propagation(&snap, cfg) {
		if !res.updates.is_empty() {
			apply_gnn_updates(q, g, kern_id, res.updates, res.weights);
		}
	}
}

pub fn build_gnn_snapshot(kern: &Kern, cfg: &GnnConfig) -> Option<GnnSnapshot> {
	if kern.entities.len() < cfg.min_thoughts {
		return None;
	}

	let mut ids = Vec::with_capacity(kern.entities.len());
	let mut dim = 0usize;
	for (id, t) in &kern.entities {
		if !t.has_vector() {
			continue;
		}
		if dim == 0 {
			dim = t.vector.len();
		}
		if t.vector.len() != dim || dim == 0 {
			continue;
		}
		ids.push(id.clone());
	}
	if ids.len() < cfg.min_thoughts || dim == 0 {
		return None;
	}

	let id_to_idx: HashMap<&str, usize> = ids
		.iter()
		.enumerate()
		.map(|(i, id)| (id.as_str(), i))
		.collect();
	let mut gg = Graph::new();
	let mut feat_data = vec![0.0f64; ids.len() * dim];

	for (i, id) in ids.iter().enumerate() {
		let t = &kern.entities[id];
		feat_data[i * dim..(i + 1) * dim].copy_from_slice(&t.vector);
		let _ = gg.add_node(id, t.vector.clone());
	}

	let mut pair_seen = HashSet::new();
	let mut pos_edges: Vec<[usize; 2]> = Vec::new();

	// GNN propagation is per-kern-local by design. Reasons whose `to`
	// endpoint lives in a different kern (`to_kern_id` non-empty) are
	// skipped: their target embedding is not in this kern's `feat_data`
	// matrix, and `gnn_vector` is not federated by gossip
	// (docs/kern/crdts-federation.md §7 lists it as explicitly excluded
	// from CRDT replication). Local model, local edges. Commit a29ea34
	// stamps `to_kern_id` more aggressively on `move_entity`, which
	// increases the count of skipped reasons here — that's the intended
	// outcome, not a regression.
	for r in kern.reasons.values() {
		if !r.to_kern_id.is_empty() || r.to.is_empty() {
			continue;
		}
		let i = match id_to_idx.get(r.from.as_str()) {
			Some(&i) => i,
			None => continue,
		};
		let j = match id_to_idx.get(r.to.as_str()) {
			Some(&j) => j,
			None => continue,
		};
		if i == j {
			continue;
		}

		let _ = gg.add_edge(&r.from, &r.to, Vec::new());
		let _ = gg.add_edge(&r.to, &r.from, Vec::new());

		let (a, b) = if i < j { (i, j) } else { (j, i) };
		if pair_seen.insert((a, b)) {
			pos_edges.push([a, b]);
		}
	}
	if pos_edges.is_empty() {
		return None;
	}
	gg.add_self_loops();

	let features = gg.feature_matrix();

	Some(GnnSnapshot {
		ids,
		features,
		graph: gg,
		pos_edges,
		weights: kern.gnn_weights.clone(),
	})
}

fn apply_gnn_updates(
	q: &Queue,
	g: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	updates: HashMap<String, Vec<f64>>,
	weights: Vec<u8>,
) {
	if updates.is_empty() {
		return;
	}
	let mut graph = write_recovered(g);
	let mut changed: Vec<(String, Vec<f64>)> = Vec::new();
	if let Some(kern) = graph.kerns.get_mut(kern_id) {
		for (entity_id, vec) in &updates {
			if vec.is_empty() {
				continue;
			}
			if let Some(t) = kern.entities.get_mut(entity_id) {
				let w = cosine_align(&t.vector, vec);
				if w >= 0.5 {
					t.observe_support(w);
				} else {
					t.observe_contradict(1.0 - w);
				}
				t.gnn_vector = vec.clone();
				changed.push((entity_id.clone(), vec.clone()));
			}
		}
		if !weights.is_empty() {
			kern.gnn_weights = weights.clone();
		}
	}
	for (id, vec) in &changed {
		graph.gnn_entity_idx.delete(id);
		graph.gnn_entity_idx.insert(id.clone(), vec.clone());
	}
	drop(graph);

	if !changed.is_empty() || !weights.is_empty() {
		q.enqueue(task(TaskKind::Persist, kern_id));
	}
}

fn cosine_align(a: &[f64], b: &[f64]) -> f64 {
	if a.is_empty() || b.is_empty() || a.len() != b.len() {
		return 0.5;
	}
	let mut dot = 0.0;
	let mut na = 0.0;
	let mut nb = 0.0;
	for i in 0..a.len() {
		dot += a[i] * b[i];
		na += a[i] * a[i];
		nb += b[i] * b[i];
	}
	if na == 0.0 || nb == 0.0 {
		return 0.5;
	}
	let cos = dot / (na.sqrt() * nb.sqrt());
	((cos + 1.0) * 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;
	use crate::base::types::{mk_entity, EntityKind, Reason};

	/// `n` entities (each with a vector) chained by local reason edges, so a
	/// snapshot has both nodes and positive edges once the floor is cleared.
	fn kern_with_n(n: usize) -> Kern {
		let mut k = Kern::new("k", "");
		for i in 0..n {
			let id = format!("e{i}");
			k.entities
				.insert(id.clone(), mk_entity(&id, &id, 0.0, EntityKind::Claim));
		}
		for i in 0..n.saturating_sub(1) {
			let from = format!("e{i}");
			let to = format!("e{}", i + 1);
			add_reason(
				&mut k,
				Reason {
					from: from.clone(),
					to: to.clone(),
					id: format!("{from}->{to}"),
					..Default::default()
				},
			);
		}
		k
	}

	#[test]
	fn gnn_skipped_below_min_thoughts_default() {
		// Card #30: a tiny graph must NOT train a GNN under the default floor.
		let k = kern_with_n(3);
		let cfg = GnnConfig::defaults(); // min_thoughts = 128
		assert!(
			build_gnn_snapshot(&k, &cfg).is_none(),
			"3-node graph skips GNN under the default min_thoughts floor"
		);
	}

	#[test]
	fn gnn_runs_when_floor_lowered() {
		// Lowering the floor re-enables training, proving it's the floor gating.
		let k = kern_with_n(3);
		let mut cfg = GnnConfig::defaults();
		cfg.min_thoughts = 2;
		assert!(
			build_gnn_snapshot(&k, &cfg).is_some(),
			"with a low floor and local edges, a snapshot builds"
		);
	}
}
