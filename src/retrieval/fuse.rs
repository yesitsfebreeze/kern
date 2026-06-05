use crate::base::search::EntityHit;
use std::collections::HashMap;

/// Weighted Reciprocal Rank Fusion. Each input list `i` contributes
/// `weights[i] / (k_rrf + rank)` to an entity's fused score; a missing weight
/// (slice shorter than `lists`, or empty) defaults to `1.0`, which recovers
/// plain unweighted RRF. Down-weighting query-INDEPENDENT lists (global
/// importance / PageRank) keeps a popular-but-irrelevant entity from getting
/// the same boost as a query-relevant dense/lexical hit.
pub fn rrf(
	lists: &[&[EntityHit]],
	weights: &[f64],
	k_rrf: f64,
	top_k: usize,
) -> Vec<EntityHit> {
	let mut agg: HashMap<String, f64> = HashMap::new();
	for (li, list) in lists.iter().enumerate() {
		let w = weights.get(li).copied().unwrap_or(1.0);
		for (i, hit) in list.iter().enumerate() {
			let rank = (i + 1) as f64;
			let contrib = w / (k_rrf + rank);
			*agg.entry(hit.entity_id.clone()).or_insert(0.0) += contrib;
		}
	}
	let mut out: Vec<EntityHit> = agg.into_iter().map(EntityHit::from).collect();
	out.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
			.then_with(|| a.entity_id.cmp(&b.entity_id))
	});
	out.truncate(top_k);
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
}
