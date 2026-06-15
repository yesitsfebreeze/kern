use crate::base::search::EntityHit;
use std::collections::HashMap;

/// Weighted Reciprocal Rank Fusion. Each input list `i` contributes
/// `weights[i] / (k_rrf + rank)` to an entity's fused score; a missing weight
/// (slice shorter than `lists`, or empty) defaults to `1.0`, which recovers
/// plain unweighted RRF. Down-weighting query-INDEPENDENT lists (global
/// importance / PageRank) keeps a popular-but-irrelevant entity from getting
/// the same boost as a query-relevant dense/lexical hit.
pub fn rrf(lists: &[&[EntityHit]], weights: &[f64], k_rrf: f64, top_k: usize) -> Vec<EntityHit> {
	let mut agg: HashMap<String, f64> = HashMap::new();
	for (li, list) in lists.iter().enumerate() {
		let w = weights.get(li).copied().unwrap_or(1.0);
		for (i, hit) in list.iter().enumerate() {
			let rank = (i + 1) as f64;
			let contrib = w / (k_rrf + rank);
			*agg.entry(hit.entity_id.clone()).or_insert(0.0) += contrib;
		}
	}
	if top_k == 0 {
		return Vec::new();
	}
	let mut out: Vec<EntityHit> = agg.into_iter().map(EntityHit::from).collect();
	// Primary key: fused score, descending. Secondary key: entity_id, ascending —
	// a deliberate deterministic tiebreak (HashMap iteration order is not), so
	// equal-score entities always sort the same way across runs. This keeps recall
	// reproducible and tests deterministic. Because `agg`'s keys are unique entity
	// ids, this is a STRICT total order: no two distinct entries compare Equal.
	let cmp = |a: &EntityHit, b: &EntityHit| {
		crate::base::util::cmp_rank(a.score, &a.entity_id, b.score, &b.entity_id)
	};
	// Only top_k of a potentially large fused union is delivered, so partition the
	// top_k into [0, top_k) in O(n) average with select_nth instead of fully
	// sorting all n in O(n log n), then order just those survivors. The strict
	// total order makes this byte-identical to a full sort + truncate.
	if top_k < out.len() {
		out.select_nth_unstable_by(top_k - 1, &cmp);
		out.truncate(top_k);
	}
	out.sort_by(&cmp);
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hit(id: &str) -> EntityHit {
		EntityHit {
			entity_id: id.into(),
			score: 0.0,
		}
	}

	#[test]
	fn empty_weights_recovers_unweighted_rrf() {
		// Two lists, no weights → every list contributes 1/(k+rank).
		let a = [hit("x"), hit("y")];
		let b = [hit("y"), hit("z")];
		let lists: Vec<&[EntityHit]> = vec![&a, &b];
		let out = rrf(&lists, &[], 60.0, 10);
		// y appears in both lists → highest fused score.
		assert_eq!(out[0].entity_id, "y");
	}

	#[test]
	fn global_list_downweight_sinks_popular_irrelevant_entity() {
		// dense (query-relevant) ranks `rel` first; a global list ranks an
		// irrelevant `pop` first. Equal weights would tie at rank 1; a 0.5
		// global weight must put the dense hit above the global-only hit.
		let dense = [hit("rel")];
		let global = [hit("pop")];
		let lists: Vec<&[EntityHit]> = vec![&dense, &global];

		// Unweighted: tie broken by id ("pop" < "rel") → pop first.
		let unweighted = rrf(&lists, &[1.0, 1.0], 60.0, 10);
		assert_eq!(unweighted[0].entity_id, "pop", "equal weights: id tiebreak");

		// Weighted: dense 1.0 vs global 0.5 → rel outranks pop.
		let weighted = rrf(&lists, &[1.0, 0.5], 60.0, 10);
		assert_eq!(weighted[0].entity_id, "rel", "down-weighted global sinks");
		assert!(
			weighted[0].score > weighted[1].score,
			"rel strictly above pop"
		);
	}

	#[test]
	fn missing_weight_defaults_to_one() {
		// weights shorter than lists → trailing lists default to weight 1.0.
		let a = [hit("x")];
		let b = [hit("x")];
		let lists: Vec<&[EntityHit]> = vec![&a, &b];
		let out = rrf(&lists, &[1.0], 60.0, 10); // second list defaults to 1.0
		let both = rrf(&lists, &[1.0, 1.0], 60.0, 10);
		assert_eq!(out[0].score, both[0].score, "missing weight == 1.0");
	}

	#[test]
	fn equal_score_tie_broken_by_id_ascending_under_top_k() {
		// "a" and "b" each rank 1 in their own list -> identical fused score. With
		// top_k=1 the select_nth partition must still resolve the tie by id
		// ascending and keep "a"; a non-total-order comparator could keep "b".
		let la = [hit("b")];
		let lb = [hit("a")];
		let lists: Vec<&[EntityHit]> = vec![&la, &lb];
		let out = rrf(&lists, &[1.0, 1.0], 60.0, 1);
		assert_eq!(out.len(), 1, "top_k=1 keeps a single hit");
		assert_eq!(
			out[0].entity_id, "a",
			"tie resolved to id-ascending winner under truncation"
		);
	}

	#[test]
	fn top_k_truncates_and_zero_is_empty_without_panicking() {
		let a = [hit("x"), hit("y"), hit("z")];
		let lists: Vec<&[EntityHit]> = vec![&a];

		// top_k = 0 -> empty vec, no panic (truncate(0)).
		assert!(
			rrf(&lists, &[], 60.0, 0).is_empty(),
			"top_k=0 yields an empty result"
		);
		// top_k below the result count truncates to the top entries.
		assert_eq!(rrf(&lists, &[], 60.0, 2).len(), 2);
		// top_k above the result count returns all, no padding/panic.
		assert_eq!(rrf(&lists, &[], 60.0, 99).len(), 3);
	}
}
