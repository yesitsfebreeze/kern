//! Normalized Discounted Cumulative Gain (NDCG@k) for the retrieval bench.
//!
//! Relevance is **binary**: an id has gain 1 iff it appears in `expected_ids`,
//! else 0 — there are no graded judgments. DCG@k sums each top-k hit's gain
//! discounted by rank, `Σ rel_i / log2(i + 2)` (0-indexed position `i`, so the
//! first slot's discount is `log2(2) = 1`). IDCG@k is the DCG of the ideal
//! ranking — every relevant id packed at the top, capped at `k`. NDCG = DCG/IDCG
//! ∈ [0, 1]: 1.0 means every expected id that *can* fit within `k` is ranked
//! above every non-relevant one.

use std::collections::HashSet;

pub fn ndcg_at_k(ranked_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
	if expected_ids.is_empty() || k == 0 {
		return 0.0;
	}
	let expected: HashSet<&str> = expected_ids.iter().map(String::as_str).collect();

	let mut dcg = 0.0;
	for (i, id) in ranked_ids.iter().take(k).enumerate() {
		if expected.contains(id.as_str()) {
			dcg += 1.0 / ((i + 2) as f64).log2();
		}
	}

	let ideal_hits = expected_ids.len().min(k);
	let mut idcg = 0.0;
	for i in 0..ideal_hits {
		idcg += 1.0 / ((i + 2) as f64).log2();
	}
	if idcg == 0.0 {
		return 0.0;
	}
	dcg / idcg
}

pub fn mean_ndcg<I>(results: I, k: usize) -> f64
where
	I: IntoIterator<Item = (Vec<String>, Vec<String>)>,
{
	let mut sum = 0.0;
	let mut n = 0;
	for (ranked, expected) in results {
		sum += ndcg_at_k(&ranked, &expected, k);
		n += 1;
	}
	if n == 0 { 0.0 } else { sum / n as f64 }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ids(xs: &[&str]) -> Vec<String> {
		xs.iter().map(|s| s.to_string()).collect()
	}

	#[test]
	fn perfect_ranking_is_one() {
		let r = ndcg_at_k(&ids(&["a", "b", "c"]), &ids(&["a", "b", "c"]), 3);
		assert!((r - 1.0).abs() < 1e-9, "all relevant, ideally ordered -> 1.0, got {r}");
	}

	#[test]
	fn zero_overlap_is_zero() {
		assert_eq!(ndcg_at_k(&ids(&["x", "y"]), &ids(&["a", "b"]), 2), 0.0);
	}

	#[test]
	fn partial_hit_matches_the_formula() {
		// ranked [a, x, b], expected {a, b}, k=3.
		// DCG  = 1/log2(2) + 1/log2(4) = 1.0 + 0.5 = 1.5  (a@0, b@2)
		// IDCG = 1/log2(2) + 1/log2(3) = 1.0 + 0.63093 = 1.63093  (2 ideal hits)
		let r = ndcg_at_k(&ids(&["a", "x", "b"]), &ids(&["a", "b"]), 3);
		let expected = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
		assert!((r - expected).abs() < 1e-9, "got {r}, want {expected}");
	}

	#[test]
	fn rank_position_matters() {
		// Same single hit, earlier position scores strictly higher.
		let high = ndcg_at_k(&ids(&["a", "x", "y"]), &ids(&["a"]), 3);
		let low = ndcg_at_k(&ids(&["x", "y", "a"]), &ids(&["a"]), 3);
		assert!((high - 1.0).abs() < 1e-9, "hit at rank 0 -> 1.0");
		assert!(low < high, "hit deeper in the list is discounted ({low} < {high})");
	}

	#[test]
	fn empty_expected_or_zero_k_is_zero() {
		assert_eq!(ndcg_at_k(&ids(&["a"]), &[], 5), 0.0, "no expected ids");
		assert_eq!(ndcg_at_k(&ids(&["a"]), &ids(&["a"]), 0), 0.0, "k=0");
	}

	#[test]
	fn k_caps_both_dcg_and_idcg() {
		// expected has 3 ids but k=1: only the top slot counts, and IDCG is also
		// capped at 1 hit, so a top-1 relevant result is a perfect 1.0.
		let r = ndcg_at_k(&ids(&["a", "b", "c"]), &ids(&["a", "b", "c"]), 1);
		assert!((r - 1.0).abs() < 1e-9, "got {r}");
	}

	#[test]
	fn mean_ndcg_averages_per_query_scores() {
		// One perfect query (1.0) + one zero-overlap query (0.0) -> mean 0.5.
		let results = vec![
			(ids(&["a"]), ids(&["a"])),
			(ids(&["x"]), ids(&["a"])),
		];
		assert!((mean_ndcg(results, 3) - 0.5).abs() < 1e-9);
		// Empty input -> 0.0, no divide-by-zero.
		let empty: Vec<(Vec<String>, Vec<String>)> = Vec::new();
		assert_eq!(mean_ndcg(empty, 3), 0.0);
	}
}
