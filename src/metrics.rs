//! Stigmergy observability metrics.
//!
//! Computes a Gini coefficient over the per-`Entity` heat distribution to
//! signal convergence of the self-organising attention field. Per
//! `docs/kern/stigmergy-self-improving.md`, sustained `G >= 0.6` indicates a
//! concentrated, self-organising regime.

use crate::base::types::Entity;
use crate::base::util::cmp_partial;

/// Snapshot of stigmergy state at a single observation point.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StigmergySnapshot {
	pub gini: f64,
	pub max_heat: f64,
	pub mean_heat: f64,
	pub n: usize,
}

/// Gini coefficient of a non-negative distribution.
///
/// Standard formula on sorted values:
///   G = (sum_{i=1..n} (2i - n - 1) * x_i) / (n * sum(x))
///
/// Edge cases: empty / single value / all zeros → `0.0`.
/// Negative inputs are clamped to `0.0` (heat is non-negative by definition;
/// debug builds assert this).
///
/// Complexity: `O(n log n)` due to the sort. Allocates exactly one `Vec<f64>`.
pub fn gini_from_iter<I: IntoIterator<Item = f64>>(values: I) -> f64 {
	let mut v: Vec<f64> = values
		.into_iter()
		.map(|x| {
			debug_assert!(x >= 0.0, "gini input must be non-negative, got {x}");
			if x < 0.0 {
				0.0
			} else {
				x
			}
		})
		.collect();

	let n = v.len();
	if n < 2 {
		return 0.0;
	}

	v.sort_by(cmp_partial);

	let sum: f64 = v.iter().sum();
	if sum <= 0.0 {
		return 0.0;
	}

	let n_f = n as f64;
	let mut weighted = 0.0;
	for (i, x) in v.iter().enumerate() {
		// 1-based index in textbook formula
		let rank = (i + 1) as f64;
		weighted += (2.0 * rank - n_f - 1.0) * x;
	}
	weighted / (n_f * sum)
}

/// Build a [`StigmergySnapshot`] from a stream of `Entity` references.
///
/// Reads `thought.heat` (`f32`) — verified present in `base::types::Entity`.
pub fn snapshot_heat<'a>(thoughts: impl IntoIterator<Item = &'a Entity>) -> StigmergySnapshot {
	let heats: Vec<f64> = thoughts.into_iter().map(|t| t.heat as f64).collect();
	let n = heats.len();
	if n == 0 {
		return StigmergySnapshot::default();
	}
	let sum: f64 = heats.iter().sum();
	let max_heat = heats.iter().copied().fold(0.0_f64, f64::max);
	let mean_heat = sum / n as f64;
	let gini = gini_from_iter(heats.iter().copied());
	StigmergySnapshot {
		gini,
		max_heat,
		mean_heat,
		n,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gini_of_empty_is_zero() {
		assert_eq!(gini_from_iter(std::iter::empty::<f64>()), 0.0);
	}

	#[test]
	fn gini_of_single_value_is_zero() {
		assert_eq!(gini_from_iter([42.0]), 0.0);
	}

	#[test]
	fn gini_of_all_zeros_is_zero() {
		assert_eq!(gini_from_iter([0.0, 0.0, 0.0, 0.0]), 0.0);
	}

	#[test]
	fn gini_of_perfectly_equal_distribution_is_zero() {
		let g = gini_from_iter([5.0, 5.0, 5.0, 5.0, 5.0]);
		assert!(g.abs() < 1e-12, "expected 0, got {g}");
	}

	#[test]
	fn gini_of_single_hot_outlier_approaches_one() {
		// One element holds all mass among n=1000.
		let mut v = vec![0.0_f64; 999];
		v.push(1000.0);
		let g = gini_from_iter(v);
		// Theoretical maximum is (n-1)/n = 0.999.
		assert!(g > 0.99, "expected near 1, got {g}");
	}

	#[test]
	fn gini_textbook_example_matches_known_value() {
		// Wikipedia worked example: incomes [1, 2, 3, 4, 5] → G = 0.2666...
		let g = gini_from_iter([1.0, 2.0, 3.0, 4.0, 5.0]);
		assert!((g - 4.0 / 15.0).abs() < 1e-12, "got {g}");
	}

	#[test]
	fn gini_clamps_negative_inputs_in_release() {
		// In release this must not panic and treats negatives as 0.
		// (debug_assert fires only in debug; this test runs in both, so we
		// only assert the release-equivalent behaviour via positives.)
		let g = gini_from_iter([0.0, 0.0, 10.0]);
		// Two zeros and one ten among n=3 → G = 2/3.
		assert!((g - 2.0 / 3.0).abs() < 1e-12, "got {g}");
	}

	#[test]
	fn snapshot_heat_handles_empty() {
		let s = snapshot_heat(std::iter::empty::<&Entity>());
		assert_eq!(s, StigmergySnapshot::default());
	}

	#[test]
	fn snapshot_heat_computes_fields() {
		let t1 = Entity {
			heat: 1.0,
			..Default::default()
		};
		let t2 = Entity {
			heat: 3.0,
			..Default::default()
		};
		let s = snapshot_heat([&t1, &t2]);
		assert_eq!(s.n, 2);
		assert_eq!(s.max_heat, 3.0);
		assert_eq!(s.mean_heat, 2.0);
		// [1,3] → G = 0.25
		assert!((s.gini - 0.25).abs() < 1e-12, "got {}", s.gini);
	}
}
