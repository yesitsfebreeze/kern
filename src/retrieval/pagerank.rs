use crate::base::graph::GraphGnn;
use crate::base::search::EntityHit;
use std::collections::HashMap;

/// Personalization/teleport distribution over the `n` nodes: seed scores
/// (clamped non-negative) normalized to sum 1. With no usable seeds, falls back
/// to uniform — i.e. global (non-personalized) PageRank.
fn teleport_vector(seeds: &[EntityHit], id_to_idx: &HashMap<String, usize>, n: usize) -> Vec<f64> {
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
		let u = 1.0 / (n as f64);
		for t in tele.iter_mut() {
			*t = u;
		}
	}
	tele
}

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
	// The dense node index + adjacency are epoch-cached on the graph: retrieval
	// runs many queries between writes, so the rebuild (String clones for every
	// entity and edge) happens once per mutation instead of once per query.
	let adj = g.entity_adjacency();
	let ids = &adj.ids;
	let n = ids.len();
	if n == 0 {
		return Vec::new();
	}
	let out = &adj.out;
	let d = damping.clamp(0.0, 1.0);
	let tele = teleport_vector(seeds, &adj.id_to_idx, n);

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
		let delta: f64 = next
			.iter()
			.zip(rank.iter())
			.map(|(a, b)| (a - b).abs())
			.sum();
		std::mem::swap(&mut rank, &mut next);
		if delta < CONVERGENCE_EPS {
			break;
		}
	}

	let take = top_k.min(n);
	if take == 0 {
		return Vec::new();
	}
	let mut scored: Vec<(usize, f64)> = rank.iter().copied().enumerate().collect();
	// Rank order: score descending, ties broken by id ascending. Because `ids` are
	// unique (deduped above), this is a STRICT total order — no two distinct
	// entries compare Equal — so a top-k partition followed by sorting only those k
	// yields exactly the same result as a full sort + take.
	let cmp = |a: &(usize, f64), b: &(usize, f64)| {
		crate::base::util::cmp_rank(a.1, &ids[a.0], b.1, &ids[b.0])
	};
	// Partition the top-k into [0, take) in O(n) average instead of fully sorting
	// all n entities in O(n log n). pagerank runs per query over the entire entity
	// graph, so this matters as the graph grows toward Qdrant scale; the final
	// sort then orders only the k survivors.
	if take < scored.len() {
		scored.select_nth_unstable_by(take - 1, &cmp);
		scored.truncate(take);
	}
	scored.sort_by(&cmp);

	let mut out_list: Vec<EntityHit> = Vec::with_capacity(take);
	for (idx, score) in scored {
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
	use crate::base::types::Kern;

	use crate::test_support::{edge, entity as ent};

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
			assert!(
				(a.score - b.score).abs() < 1e-6,
				"converged result is iteration-count-independent"
			);
		}
	}

	#[test]
	fn top_k_partition_matches_full_sort_prefix() {
		// Distinct in-degrees -> distinct ranks. A top_k=3 query must return the
		// exact same first three (id and score) as the full ranking, proving the
		// select_nth_unstable_by partition agrees with a full sort + take.
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		let nodes = ["A", "B", "C", "D", "E", "F", "G", "H"];
		for id in nodes {
			k.entities.insert(id.into(), ent(id));
		}
		for e in [
			edge("A", "D"),
			edge("B", "D"),
			edge("C", "D"),
			edge("D", "E"),
			edge("F", "E"),
			edge("G", "E"),
			edge("H", "A"),
		] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);
		let full = pagerank(&g, &[], 0.85, 200, nodes.len());
		let topk = pagerank(&g, &[], 0.85, 200, 3);
		assert_eq!(topk.len(), 3, "top_k truncates to 3");
		for i in 0..3 {
			assert_eq!(
				topk[i].entity_id, full[i].entity_id,
				"top-{i} id matches full prefix"
			);
			assert!(
				(topk[i].score - full[i].score).abs() < 1e-12,
				"top-{i} score matches"
			);
		}
	}

	#[test]
	fn ties_break_by_id_ascending_under_top_k() {
		// No edges -> uniform rank -> every node ties. With top_k=1 the partition
		// must still resolve the tie by id ascending ("A"), not by hash/insertion
		// order. This is the case that would break if the comparator were not a
		// strict total order through select_nth_unstable_by.
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for id in ["C", "A", "B"] {
			k.entities.insert(id.into(), ent(id));
		}
		g.register(k);
		let r = pagerank(&g, &[], 0.85, 50, 1);
		assert_eq!(r.len(), 1);
		assert_eq!(
			r[0].entity_id, "A",
			"tied ranks resolve to the id-ascending winner"
		);
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
