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

pub fn recall_at_k(ranked_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
	if expected_ids.is_empty() || k == 0 {
		return 0.0;
	}
	let expected: HashSet<&str> = expected_ids.iter().map(String::as_str).collect();
	let top: HashSet<&str> = ranked_ids.iter().take(k).map(String::as_str).collect();
	expected.intersection(&top).count() as f64 / expected.len() as f64
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
		assert!(
			(r - 1.0).abs() < 1e-9,
			"all relevant, ideally ordered -> 1.0, got {r}"
		);
	}

	#[test]
	fn zero_overlap_is_zero() {
		assert_eq!(ndcg_at_k(&ids(&["x", "y"]), &ids(&["a", "b"]), 2), 0.0);
	}

	#[test]
	fn partial_hit_matches_the_formula() {
		let r = ndcg_at_k(&ids(&["a", "x", "b"]), &ids(&["a", "b"]), 3);
		let expected = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
		assert!((r - expected).abs() < 1e-9, "got {r}, want {expected}");
	}

	#[test]
	fn rank_position_matters() {
		let high = ndcg_at_k(&ids(&["a", "x", "y"]), &ids(&["a"]), 3);
		let low = ndcg_at_k(&ids(&["x", "y", "a"]), &ids(&["a"]), 3);
		assert!((high - 1.0).abs() < 1e-9, "hit at rank 0 -> 1.0");
		assert!(
			low < high,
			"hit deeper in the list is discounted ({low} < {high})"
		);
	}

	#[test]
	fn empty_expected_or_zero_k_is_zero() {
		assert_eq!(ndcg_at_k(&ids(&["a"]), &[], 5), 0.0, "no expected ids");
		assert_eq!(ndcg_at_k(&ids(&["a"]), &ids(&["a"]), 0), 0.0, "k=0");
	}

	#[test]
	fn k_caps_both_dcg_and_idcg() {
		let r = ndcg_at_k(&ids(&["a", "b", "c"]), &ids(&["a", "b", "c"]), 1);
		assert!((r - 1.0).abs() < 1e-9, "got {r}");
	}

	#[test]
	fn recall_counts_coverage_ignoring_order() {
		assert_eq!(
			recall_at_k(&ids(&["x", "y", "a", "b"]), &ids(&["a", "b"]), 4),
			1.0
		);
		assert_eq!(
			recall_at_k(&ids(&["a", "x", "y"]), &ids(&["a", "b"]), 3),
			0.5
		);
		assert_eq!(recall_at_k(&ids(&["x", "y"]), &ids(&["a", "b"]), 2), 0.0);
	}

	#[test]
	fn recall_is_bounded_by_k_and_never_exceeds_one() {
		let r = recall_at_k(&ids(&["a", "b", "c"]), &ids(&["a", "b", "c"]), 1);
		assert!(
			(r - 1.0 / 3.0).abs() < 1e-9,
			"k caps reachable recall, got {r}"
		);
		assert_eq!(recall_at_k(&ids(&["a", "a"]), &ids(&["a"]), 2), 1.0);
	}

	#[test]
	fn recall_empty_expected_or_zero_k_is_zero() {
		assert_eq!(recall_at_k(&ids(&["a"]), &[], 5), 0.0);
		assert_eq!(recall_at_k(&ids(&["a"]), &ids(&["a"]), 0), 0.0);
	}
}
