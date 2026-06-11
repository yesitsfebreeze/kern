use crate::base::graph::GraphGnn;
use crate::base::search::EntityHit;
use std::collections::HashMap;

/// PageRank over the entity graph.
///
/// `seeds` is the personalization (teleport) distribution: when non-empty the
/// teleport vector is built from the seed entities weighted by their scores,
/// yielding query-Personalized PageRank (HippoRAG 2 — seed teleport at
/// query-linked entities for multi-hop / associative recall). When `seeds` is
/// empty (or none of the seeds exist in the graph) the teleport is uniform,
/// recovering global query-independent PageRank.
pub fn pagerank(
	g: &GraphGnn,
	seeds: &[EntityHit],
	damping: f64,
	iters: usize,
	top_k: usize,
) -> Vec<EntityHit> {
	let mut id_to_idx: HashMap<String, usize> = HashMap::new();
	let mut ids: Vec<String> = Vec::new();
	for kern in g.map().values() {
		for t in kern.entities.values() {
			if !id_to_idx.contains_key(&t.id) {
				id_to_idx.insert(t.id.clone(), ids.len());
				ids.push(t.id.clone());
			}
		}
	}
	let n = ids.len();
	if n == 0 {
		return Vec::new();
	}

	let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
	for kern in g.map().values() {
		for r in kern.reasons.values() {
			if r.from == r.to {
				continue;
			}
			let (Some(&fi), Some(&ti)) = (id_to_idx.get(&r.from), id_to_idx.get(&r.to)) else {
				continue;
			};
			out[fi].push(ti);
		}
	}

	let d = damping.clamp(0.0, 1.0);

	// Personalization / teleport distribution.
	let mut tele = vec![0.0f64; n];
	let mut seed_sum = 0.0;
	for s in seeds {
		if let Some(&i) = id_to_idx.get(&s.entity_id) {
			let w = s.score.max(0.0);
			tele[i] += w;
			seed_sum += w;
		}
	}
	if seed_sum > 0.0 {
		for t in tele.iter_mut() {
			*t /= seed_sum;
		}
	} else {
		// No usable seeds → uniform teleport = global PageRank.
		let u = 1.0 / (n as f64);
		for t in tele.iter_mut() {
			*t = u;
		}
	}

	let mut rank = tele.clone();
	let mut next = vec![0.0f64; n];

	// Stop early once the rank vector stops moving — `iters` is just an upper
	// bound. Power iteration on a stochastic matrix converges geometrically, so a
	// well-connected graph typically settles in far fewer than the cap.
	const CONVERGENCE_EPS: f64 = 1e-9;

	for _ in 0..iters.max(1) {
		let mut dangling = 0.0;
		for (j, outs) in out.iter().enumerate() {
			if outs.is_empty() {
				dangling += rank[j];
			}
		}
		// Dangling mass is redistributed along the teleport vector so the
		// personalization bias is preserved (not leaked uniformly).
		let dangling_mass = d * dangling;
		let base = 1.0 - d + dangling_mass;

		for (i, slot) in next.iter_mut().enumerate() {
			*slot = base * tele[i];
		}
		for (j, outs) in out.iter().enumerate() {
			if outs.is_empty() {
				continue;
			}
			let share = d * rank[j] / (outs.len() as f64);
			for &ti in outs {
				next[ti] += share;
			}
		}
		// L1 movement this step; once below epsilon the ranks have converged and
		// further iterations only re-derive the same fixed point.
		let delta: f64 = next.iter().zip(rank.iter()).map(|(a, b)| (a - b).abs()).sum();
		std::mem::swap(&mut rank, &mut next);
		if delta < CONVERGENCE_EPS {
			break;
		}
	}

	let mut scored: Vec<(usize, f64)> = rank.iter().copied().enumerate().collect();
	scored.sort_by(|a, b| {
		b.1
			.partial_cmp(&a.1)
			.unwrap_or(std::cmp::Ordering::Equal)
			.then_with(|| ids[a.0].cmp(&ids[b.0]))
	});

	let take = top_k.min(n);
	let mut out_list: Vec<EntityHit> = Vec::with_capacity(take);
	for (idx, score) in scored.into_iter().take(take) {
		out_list.push(EntityHit {
			entity_id: ids[idx].clone(),
			score,
		});
	}
	out_list
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Kern, Reason};

	fn ent(id: &str) -> Entity {
		Entity {
			id: id.into(),
			..Default::default()
		}
	}
	fn edge(from: &str, to: &str) -> Reason {
		Reason {
			from: from.into(),
			to: to.into(),
			id: format!("{from}->{to}"),
			..Default::default()
		}
	}

	fn hit(id: &str, score: f64) -> EntityHit {
		EntityHit {
			entity_id: id.into(),
			score,
		}
	}

	#[test]
	fn empty_graph_is_empty() {
		assert!(pagerank(&GraphGnn::new(), &[], 0.85, 10, 5).is_empty());
	}

	#[test]
	fn ranks_hub_above_leaves_and_sums_to_one() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for id in ["A", "B", "C"] {
			k.entities.insert(id.into(), ent(id));
		}
		// B -> A and C -> A : A is the hub.
		for e in [edge("B", "A"), edge("C", "A")] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);

		// Empty seeds → uniform teleport → global PageRank.
		let ranks = pagerank(&g, &[], 0.85, 100, 3);
		assert_eq!(ranks.len(), 3);
		let score = |id: &str| ranks.iter().find(|h| h.entity_id == id).unwrap().score;
		assert!(score("A") > score("B"), "hub A must outrank leaf B");
		let sum: f64 = ranks.iter().map(|h| h.score).sum();
		assert!((sum - 1.0).abs() < 1e-6, "ranks sum ~1, got {sum}");
	}

	#[test]
	fn self_loops_do_not_inflate_score() {
		// A self-loop `A -> A` is dropped during adjacency build (from == to), so
		// A's rank is identical whether or not the loop is present.
		let make = |with_loop: bool| {
			let mut g = GraphGnn::new();
			let mut k = Kern::new("k", "");
			for id in ["A", "B"] {
				k.entities.insert(id.into(), ent(id));
			}
			k.reasons.insert("B->A".into(), edge("B", "A"));
			if with_loop {
				k.reasons.insert("A->A".into(), edge("A", "A"));
			}
			g.register(k);
			pagerank(&g, &[], 0.85, 100, 2)
		};
		let s = |v: &[EntityHit], id: &str| v.iter().find(|h| h.entity_id == id).unwrap().score;
		let base = make(false);
		let looped = make(true);
		assert!(
			(s(&base, "A") - s(&looped, "A")).abs() < 1e-9,
			"self-loop must not change A's rank ({} vs {})",
			s(&base, "A"),
			s(&looped, "A")
		);
	}

	#[test]
	fn convergence_early_exit_matches_full_iteration() {
		// With the L1 early-exit, a tiny well-connected graph converges in far
		// fewer than the cap; a 5-iter run must equal a 1000-iter run.
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for id in ["A", "B", "C"] {
			k.entities.insert(id.into(), ent(id));
		}
		for e in [edge("A", "B"), edge("B", "C"), edge("C", "A")] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);
		let few = pagerank(&g, &[], 0.85, 5, 3);
		let many = pagerank(&g, &[], 0.85, 1000, 3);
		for (a, b) in few.iter().zip(many.iter()) {
			assert_eq!(a.entity_id, b.entity_id);
			assert!((a.score - b.score).abs() < 1e-6, "converged result is iteration-count-independent");
		}
	}

	#[test]
	fn personalization_biases_toward_seed_and_conserves_mass() {
		// Two disconnected components: A<-B and X<-Y. Without seeds the two
		// hubs A and X are symmetric. Seeding Y must lift the X-component.
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for id in ["A", "B", "X", "Y"] {
			k.entities.insert(id.into(), ent(id));
		}
		for e in [edge("B", "A"), edge("Y", "X")] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);

		let global = pagerank(&g, &[], 0.85, 200, 4);
		let gscore = |id: &str| global.iter().find(|h| h.entity_id == id).unwrap().score;
		// Symmetric components: A and X tie globally.
		assert!((gscore("A") - gscore("X")).abs() < 1e-6, "A,X symmetric");

		let seeded = pagerank(&g, &[hit("Y", 1.0)], 0.85, 200, 4);
		let sscore = |id: &str| seeded.iter().find(|h| h.entity_id == id).unwrap().score;
		// Seeding Y pushes mass into the X-component (Y and its target X).
		assert!(sscore("X") > gscore("X"), "seeded X must beat global X");
		assert!(sscore("X") > sscore("A"), "seeded X-component outranks A");
		let sum: f64 = seeded.iter().map(|h| h.score).sum();
		assert!((sum - 1.0).abs() < 1e-6, "seeded ranks sum ~1, got {sum}");
	}
}
