use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::types::{EntityStatus, Kern};
use crate::gnn::graph::Graph;
use crate::gnn::propagate::{self, GnnConfig, GnnSnapshot};

use super::queue::{task, Queue, TaskKind};

pub fn do_gnn_propagate(q: &Queue, g: &Arc<RwLock<GraphGnn>>, kern_id: &str, cfg: &GnnConfig) {
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
		// Superseded entities are excluded: propagating would RE-INSERT them into
		// gnn_entity_idx via `apply_gnn_updates`, undoing the supersede removal.
		if t.status == EntityStatus::Superseded {
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
	for id in &ids {
		let t = &kern.entities[id];
		let feat: Vec<f64> = t.vector.iter().map(|&x| x as f64).collect();
		let _ = gg.add_node(id, feat);
	}

	let mut pair_seen = HashSet::new();
	let mut pos_edges: Vec<[usize; 2]> = Vec::new();

	// Cross-kern reasons (`to_kern_id` non-empty) are skipped by design — their
	// target embedding is not in this kern's snapshot.
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
	let mut changed: Vec<(String, Vec<f32>)> = Vec::new();
	if let Some(kern) = graph.kerns.get_mut(kern_id) {
		for (entity_id, vec) in &updates {
			if vec.is_empty() {
				continue;
			}
			if let Some(t) = kern.entities.get_mut(entity_id) {
				let vec32: Vec<f32> = vec.iter().map(|&x| x as f32).collect();
				let w = cosine_align(&t.vector, &vec32);
				if w >= 0.5 {
					t.observe_support(w);
				} else {
					t.observe_contradict(1.0 - w);
				}
				t.gnn_vector = vec32.clone();
				changed.push((entity_id.clone(), vec32));
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

/// Cosine mapped into a `[0,1]` alignment weight: identical → 1.0, opposite → 0.0,
/// orthogonal/degenerate (zero-norm, length mismatch) → 0.5 neutral.
fn cosine_align(a: &[f32], b: &[f32]) -> f64 {
	if a.is_empty() || b.is_empty() || a.len() != b.len() {
		return 0.5;
	}
	let cos = crate::base::math::cosine(a, b);
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
		let k = kern_with_n(3);
		let cfg = GnnConfig::defaults(); // min_thoughts = 128
		assert!(
			build_gnn_snapshot(&k, &cfg).is_none(),
			"3-node graph skips GNN under the default min_thoughts floor"
		);
	}

	#[test]
	fn gnn_runs_when_floor_lowered() {
		let k = kern_with_n(3);
		let mut cfg = GnnConfig::defaults();
		cfg.min_thoughts = 2;
		assert!(
			build_gnn_snapshot(&k, &cfg).is_some(),
			"with a low floor and local edges, a snapshot builds"
		);
	}

	#[test]
	fn superseded_entities_excluded_from_gnn_snapshot() {
		// Supersede a LEAF (e3) so e0->e1->e2 stay connected.
		let mut k = kern_with_n(4);
		k.entities.get_mut("e3").unwrap().status = EntityStatus::Superseded;
		let mut cfg = GnnConfig::defaults();
		cfg.min_thoughts = 2;
		let snap = build_gnn_snapshot(&k, &cfg).expect("active e0..e2 still build a snapshot");
		assert!(
			!snap.ids.contains(&"e3".to_string()),
			"superseded leaf excluded from GNN membership"
		);
		for id in ["e0", "e1", "e2"] {
			assert!(snap.ids.contains(&id.to_string()), "active {id} included");
		}
	}

	#[test]
	fn cosine_align_maps_similarity_into_zero_one() {
		assert_eq!(
			cosine_align(&[1.0, 0.0], &[1.0, 0.0]),
			1.0,
			"identical -> 1.0"
		);
		assert_eq!(
			cosine_align(&[1.0, 0.0], &[-1.0, 0.0]),
			0.0,
			"opposite -> 0.0"
		);
		assert!(
			(cosine_align(&[1.0, 0.0], &[0.0, 1.0]) - 0.5).abs() < 1e-6,
			"orthogonal -> 0.5"
		);
		assert_eq!(cosine_align(&[], &[]), 0.5, "empty -> 0.5");
		assert_eq!(
			cosine_align(&[1.0, 2.0], &[1.0]),
			0.5,
			"length mismatch -> 0.5"
		);
		assert_eq!(
			cosine_align(&[0.0, 0.0], &[1.0, 1.0]),
			0.5,
			"zero-norm -> 0.5"
		);
	}

	#[test]
	fn apply_gnn_updates_writes_gnn_vector_weights_and_enqueues_persist() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities
			.insert("e0".into(), mk_entity("e0", "e0", 0.0, EntityKind::Claim));
		g.kerns.insert("k".into(), k);
		let g = Arc::new(RwLock::new(g));

		let new_vec = vec![0.25f64, 0.5, 0.75];
		let mut updates = HashMap::new();
		updates.insert("e0".to_string(), new_vec.clone());
		let q = Queue::new(16);

		apply_gnn_updates(&q, &g, "k", updates, vec![9, 9]);

		{
			let gg = read_recovered(&g);
			let kern = gg.kerns.get("k").unwrap();
			assert_eq!(
				kern.entities["e0"].gnn_vector,
				vec![0.25f32, 0.5, 0.75],
				"gnn_vector overwritten (narrowed at the boundary)"
			);
			assert_eq!(kern.gnn_weights, vec![9, 9], "kern gnn_weights stored");
		}

		let mut rx = q.take_receiver().unwrap();
		let mut persisted = false;
		while let Ok(t) = rx.try_recv() {
			if matches!(t.kind, TaskKind::Persist) {
				persisted = true;
			}
		}
		assert!(persisted, "a Persist task is enqueued after updates land");
	}

	#[test]
	fn apply_gnn_updates_skips_empty_update_vectors() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities
			.insert("e0".into(), mk_entity("e0", "e0", 0.0, EntityKind::Claim));
		g.kerns.insert("k".into(), k);
		let g = Arc::new(RwLock::new(g));

		let mut updates = HashMap::new();
		updates.insert("e0".to_string(), Vec::new()); // empty -> skipped
		let q = Queue::new(16);
		apply_gnn_updates(&q, &g, "k", updates, Vec::new());

		let gg = read_recovered(&g);
		assert!(
			gg.kerns["k"].entities["e0"].gnn_vector.is_empty(),
			"empty update doesn't write"
		);
	}
}
