use crate::base::graph::GraphGnn;
use crate::base::search::EntityHit;
use std::collections::HashMap;

// Returns the teleport vector and its support, ascending. The support is what
// bounds the iteration: personalized mass starts there and reaches nowhere else.
fn teleport_vector(
	seeds: &[EntityHit],
	id_to_idx: &HashMap<String, usize>,
	n: usize,
) -> (Vec<f64>, Vec<usize>) {
	let mut tele = vec![0.0f64; n];
	let mut support: Vec<usize> = Vec::with_capacity(seeds.len());
	let mut seed_sum = 0.0;
	for s in seeds {
		if let Some(&i) = id_to_idx.get(&s.entity_id) {
			let w = s.score.max(0.0);
			support.push(i);
			tele[i] += w;
			seed_sum += w;
		}
	}
	if seed_sum > 0.0 {
		support.sort_unstable();
		support.dedup();
		// Normalising only the support leaves the rest of `tele` on its untouched
		// zero pages — the last full-width pass this function used to make.
		for &i in &support {
			tele[i] /= seed_sum;
		}
		(tele, support)
	} else {
		let u = 1.0 / (n as f64);
		for t in tele.iter_mut() {
			*t = u;
		}
		(tele, (0..n).collect())
	}
}

// Both inputs ascending; `merged` is overwritten with their union, ascending.
// The sets are disjoint by construction (a node joins the reached set once).
fn merge_ascending(a: &[usize], b: &[usize], merged: &mut Vec<usize>) {
	merged.clear();
	merged.reserve(a.len() + b.len());
	let (mut i, mut j) = (0, 0);
	while i < a.len() && j < b.len() {
		if a[i] <= b[j] {
			merged.push(a[i]);
			i += 1;
		} else {
			merged.push(b[j]);
			j += 1;
		}
	}
	merged.extend_from_slice(&a[i..]);
	merged.extend_from_slice(&b[j..]);
}

pub fn pagerank(
	g: &GraphGnn,
	seeds: &[EntityHit],
	damping: f64,
	iters: usize,
	top_k: usize,
) -> Vec<EntityHit> {
	let adj = g.entity_adjacency();
	let ids = &adj.ids;
	let n = ids.len();
	if n == 0 {
		return Vec::new();
	}
	let out = &adj.out;
	let d = damping.clamp(0.0, 1.0);
	let (tele, support) = teleport_vector(seeds, &adj.id_to_idx, n);

	let mut rank = vec![0.0f64; n];
	for &i in &support {
		rank[i] = tele[i];
	}
	let mut next = vec![0.0f64; n];

	// The iteration is confined to `reached` — the teleport support plus everything
	// downstream of it — because every node outside it holds an exact 0.0 in both
	// vectors, so every term the full-width loop would add for it is +0.0. Walking
	// `reached` ascending leaves the surviving terms in the full-width loop's order,
	// which is what makes this identical to it rather than merely close.
	let mut reached = support;
	let mut in_reached = vec![false; n];
	for &i in &reached {
		in_reached[i] = true;
	}
	let mut fresh: Vec<usize> = Vec::new();
	let mut merged: Vec<usize> = Vec::new();
	let mut closed = false;

	// Stop early once the rank vector stops moving — `iters` is just an upper bound.
	const CONVERGENCE_EPS: f64 = 1e-9;

	for _ in 0..iters.max(1) {
		let mut dangling = 0.0;
		for &j in &reached {
			if out[j].is_empty() {
				dangling += rank[j];
			}
		}
		// Dangling mass redistributed along the teleport vector (NOT uniformly) so the personalization bias is preserved.
		let dangling_mass = d * dangling;
		let base = 1.0 - d + dangling_mass;

		// Everything ever written to `next` lies in `reached`, which only grows, so
		// this also clears the values left from two iterations ago.
		for &i in &reached {
			next[i] = base * tele[i];
		}
		fresh.clear();
		for &j in &reached {
			let outs = &out[j];
			if outs.is_empty() {
				continue;
			}
			let share = d * rank[j] / (outs.len() as f64);
			for &ti in outs {
				next[ti] += share;
				if !closed && !in_reached[ti] {
					in_reached[ti] = true;
					fresh.push(ti);
				}
			}
		}
		if fresh.is_empty() {
			// An iteration that reached nothing new proves the set is closed under
			// out-edges, so it can never grow again and the per-edge membership probe
			// above — the one cost this walk adds per edge — is dead weight from here on.
			closed = true;
		} else {
			fresh.sort_unstable();
			merge_ascending(&reached, &fresh, &mut merged);
			std::mem::swap(&mut reached, &mut merged);
		}
		let delta: f64 = reached.iter().map(|&i| (next[i] - rank[i]).abs()).sum();
		std::mem::swap(&mut rank, &mut next);
		if delta < CONVERGENCE_EPS {
			break;
		}
	}

	let take = top_k.min(n);
	if take == 0 {
		return Vec::new();
	}
	// Unique ids make this a STRICT total order, so the top-k partition + sorting only the survivors equals a full sort + take.
	let cmp = |a: &(usize, f64), b: &(usize, f64)| {
		crate::base::util::cmp_rank(a.1, &ids[a.0], b.1, &ids[b.0])
	};
	// A zero-rank node loses to every positive one, so once the reached set alone
	// can fill top_k the untouched majority cannot enter it and never gets scanned.
	let mut scored: Vec<(usize, f64)> = reached
		.iter()
		.filter(|&&i| rank[i] > 0.0)
		.map(|&i| (i, rank[i]))
		.collect();
	if scored.len() < take {
		scored = rank.iter().copied().enumerate().collect();
	}
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

	// The full-width power iteration this file used to run, kept verbatim as the
	// reference the confined one is checked against. Only its cost is supposed to
	// have changed, so an approximation here would check nothing.
	fn pagerank_full_width(
		g: &GraphGnn,
		seeds: &[EntityHit],
		damping: f64,
		iters: usize,
		top_k: usize,
	) -> Vec<EntityHit> {
		let adj = g.entity_adjacency();
		let ids = &adj.ids;
		let n = ids.len();
		if n == 0 {
			return Vec::new();
		}
		let out = &adj.out;
		let d = damping.clamp(0.0, 1.0);
		let mut tele = vec![0.0f64; n];
		let mut seed_sum = 0.0;
		for s in seeds {
			if let Some(&i) = adj.id_to_idx.get(&s.entity_id) {
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

		let mut rank = tele.clone();
		let mut next = vec![0.0f64; n];
		for _ in 0..iters.max(1) {
			let mut dangling = 0.0;
			for (j, outs) in out.iter().enumerate() {
				if outs.is_empty() {
					dangling += rank[j];
				}
			}
			let base = 1.0 - d + d * dangling;
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
			let delta: f64 = next
				.iter()
				.zip(rank.iter())
				.map(|(a, b)| (a - b).abs())
				.sum();
			std::mem::swap(&mut rank, &mut next);
			if delta < 1e-9 {
				break;
			}
		}
		let take = top_k.min(n);
		if take == 0 {
			return Vec::new();
		}
		let mut scored: Vec<(usize, f64)> = rank.iter().copied().enumerate().collect();
		scored.sort_by(|a, b| crate::base::util::cmp_rank(a.1, &ids[a.0], b.1, &ids[b.0]));
		scored.truncate(take);
		scored
			.into_iter()
			.map(|(idx, score)| EntityHit {
				entity_id: ids[idx].clone(),
				score,
			})
			.collect()
	}

	// Deterministic graph: `n` nodes, `fanout` out-edges each, stride-chosen so the
	// reached set is a small slice of the graph at fanout 1 and most of it at 8.
	fn synth(n: usize, fanout: usize, dangling_every: usize) -> GraphGnn {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for i in 0..n {
			k.entities
				.insert(format!("e{i:05}"), ent(&format!("e{i:05}")));
		}
		let mut h: u64 = 0x2545F491_4F6CDD1D;
		for i in 0..n {
			if i % dangling_every == 0 {
				continue;
			}
			for f in 0..fanout {
				h ^= h << 13;
				h ^= h >> 7;
				h ^= h << 17;
				let to = (h as usize) % n;
				let e = edge(&format!("e{i:05}"), &format!("e{to:05}"));
				k.reasons.insert(format!("r{i}_{f}"), e);
			}
		}
		g.register(k);
		g
	}

	// The confined iteration's win is bounded by how far the seeds reach, so the
	// number it earns is a property of the graph, not of the code. Kept as the
	// instrument for that claim.
	//
	//   cargo test --release --lib -- --ignored --nocapture pagerank::tests::cost
	#[test]
	#[ignore = "measurement, not an assertion; run explicitly in release"]
	fn cost_against_full_width_by_fanout() {
		use std::time::Instant;
		const N: usize = 100_000;
		let seeds: Vec<EntityHit> = (0..75)
			.map(|i| hit(&format!("e{:05}", i * 137), 1.0))
			.collect();
		for fanout in [1usize, 4, 16] {
			let g = synth(N, fanout, 2);
			// The adjacency is built once per graph and cached on it, so whichever call
			// ran first would otherwise be charged the whole build.
			let _ = g.entity_adjacency();
			let t = Instant::now();
			let a = pagerank(&g, &seeds, 0.85, 25, 100);
			let confined = t.elapsed().as_secs_f64() * 1000.0;
			let t = Instant::now();
			let b = pagerank_full_width(&g, &seeds, 0.85, 25, 100);
			let full = t.elapsed().as_secs_f64() * 1000.0;
			assert_eq!(a.len(), b.len());
			println!("N={N} fanout={fanout:<3} confined={confined:7.3}ms full_width={full:7.3}ms");
		}
	}

	#[test]
	fn confined_iteration_equals_the_full_width_one_bit_for_bit() {
		let mut cases = 0;
		for (n, fanout, dangling_every) in [(400usize, 1usize, 3usize), (400, 8, 7), (60, 3, 2)] {
			let g = synth(n, fanout, dangling_every);
			for seed_ids in [
				vec![],
				vec![("e00007", 1.0)],
				vec![("e00003", 0.9), ("e00011", 0.2), ("e00019", 0.4)],
				// A zero-weight seed contributes no teleport mass but still names a node.
				vec![("e00003", 0.0), ("e00003", 0.5), ("e00011", -1.0)],
			] {
				let seeds: Vec<EntityHit> = seed_ids.iter().map(|(i, s)| hit(i, *s)).collect();
				for top_k in [3usize, 100, n * 2] {
					for iters in [1usize, 4, 25] {
						let got = pagerank(&g, &seeds, 0.85, iters, top_k);
						let want = pagerank_full_width(&g, &seeds, 0.85, iters, top_k);
						assert_eq!(
							got.len(),
							want.len(),
							"n={n} fanout={fanout} top_k={top_k} iters={iters} length"
						);
						for (a, b) in got.iter().zip(want.iter()) {
							assert_eq!(
								a.entity_id, b.entity_id,
								"n={n} fanout={fanout} top_k={top_k} iters={iters} order"
							);
							assert_eq!(
								a.score.to_bits(),
								b.score.to_bits(),
								"n={n} fanout={fanout} top_k={top_k} iters={iters} score for {}: {} vs {}",
								a.entity_id,
								a.score,
								b.score
							);
						}
						cases += 1;
					}
				}
			}
		}
		assert_eq!(cases, 108, "every configuration was actually compared");
	}

	#[test]
	fn rank_reaches_past_the_seed_and_its_first_hop() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for id in ["A", "B", "C", "D", "Z"] {
			k.entities.insert(id.into(), ent(id));
		}
		for e in [edge("A", "B"), edge("B", "C"), edge("C", "D")] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);

		let r = pagerank(&g, &[hit("A", 1.0)], 0.85, 25, 5);
		let s = |id: &str| r.iter().find(|h| h.entity_id == id).unwrap().score;
		for id in ["B", "C", "D"] {
			assert!(
				s(id) > 0.0,
				"{id} is downstream of the seed and must be ranked"
			);
		}
		assert!(
			s("B") > s("C") && s("C") > s("D"),
			"rank decays with distance"
		);
		assert_eq!(s("Z"), 0.0, "Z is unreachable from the seed");
	}

	#[test]
	fn a_new_edge_changes_the_ranking_it_should_change() {
		let build = |extra: bool| {
			let mut g = GraphGnn::new();
			let mut k = Kern::new("k", "");
			for id in ["A", "B", "C"] {
				k.entities.insert(id.into(), ent(id));
			}
			k.reasons.insert("A->B".into(), edge("A", "B"));
			g.register(k);
			let before = pagerank(&g, &[hit("A", 1.0)], 0.85, 25, 3);
			if !extra {
				return before;
			}
			g.get_mut("k")
				.unwrap()
				.reasons
				.insert("A->C".into(), edge("A", "C"));
			pagerank(&g, &[hit("A", 1.0)], 0.85, 25, 3)
		};
		let s = |v: &[EntityHit], id: &str| v.iter().find(|h| h.entity_id == id).unwrap().score;
		let before = build(false);
		let after = build(true);
		assert_eq!(
			s(&before, "C"),
			0.0,
			"C is unreached before the edge exists"
		);
		assert!(
			s(&after, "C") > 0.0,
			"the edge added after the first query must be visible to the second"
		);
		assert!(
			s(&after, "B") < s(&before, "B"),
			"B now splits A's outflow with C ({} vs {})",
			s(&after, "B"),
			s(&before, "B")
		);
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
		for e in [edge("B", "A"), edge("C", "A")] {
			k.reasons.insert(e.id.clone(), e);
		}
		g.register(k);

		let ranks = pagerank(&g, &[], 0.85, 100, 3);
		assert_eq!(ranks.len(), 3);
		let score = |id: &str| ranks.iter().find(|h| h.entity_id == id).unwrap().score;
		assert!(score("A") > score("B"), "hub A must outrank leaf B");
		let sum: f64 = ranks.iter().map(|h| h.score).sum();
		assert!((sum - 1.0).abs() < 1e-6, "ranks sum ~1, got {sum}");
	}

	#[test]
	fn self_loops_do_not_inflate_score() {
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
		assert!((gscore("A") - gscore("X")).abs() < 1e-6, "A,X symmetric");

		let seeded = pagerank(&g, &[hit("Y", 1.0)], 0.85, 200, 4);
		let sscore = |id: &str| seeded.iter().find(|h| h.entity_id == id).unwrap().score;
		assert!(sscore("X") > gscore("X"), "seeded X must beat global X");
		assert!(sscore("X") > sscore("A"), "seeded X-component outranks A");
		let sum: f64 = seeded.iter().map(|h| h.score).sum();
		assert!((sum - 1.0).abs() < 1e-6, "seeded ranks sum ~1, got {sum}");
	}
}
