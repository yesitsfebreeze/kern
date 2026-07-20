//! Uncertainty for eval scores: a score without an interval invites reading
//! noise as signal, and a paired A/B without a test invites shipping it.

/// Wilson score interval — correct near 0 and 1 where the normal approximation
/// produces impossible bounds, which matters here since categories score ~0.05.
pub fn wilson(successes: usize, n: usize, z: f64) -> (f64, f64) {
	if n == 0 {
		return (0.0, 0.0);
	}
	let n_f = n as f64;
	let p = successes as f64 / n_f;
	let z2 = z * z;
	let denom = 1.0 + z2 / n_f;
	let center = p + z2 / (2.0 * n_f);
	let margin = z * ((p * (1.0 - p) / n_f) + z2 / (4.0 * n_f * n_f)).sqrt();
	(
		((center - margin) / denom).max(0.0),
		((center + margin) / denom).min(1.0),
	)
}

pub const Z95: f64 = 1.96;

/// Two-sided exact binomial p-value for McNemar's test on paired outcomes.
/// `a_only`/`b_only` are the discordant counts; concordant pairs carry no
/// information about a difference, so they are deliberately not arguments.
pub fn mcnemar_exact(a_only: usize, b_only: usize) -> f64 {
	let n = a_only + b_only;
	if n == 0 {
		return 1.0;
	}
	let k = a_only.min(b_only);
	// Sum the tail then double it; clamp because the doubled tail can exceed 1
	// when the split is near even.
	let mut tail = 0.0;
	for i in 0..=k {
		tail += binom(n, i);
	}
	let total = 2f64.powi(n as i32);
	((2.0 * tail) / total).min(1.0)
}

fn binom(n: usize, k: usize) -> f64 {
	let mut acc = 1.0;
	for i in 0..k {
		acc = acc * (n - i) as f64 / (i + 1) as f64;
	}
	acc
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wilson_is_inside_the_unit_interval_at_the_extremes() {
		let (lo, hi) = wilson(0, 100, Z95);
		assert_eq!(lo, 0.0, "zero successes cannot go below 0");
		assert!(hi > 0.0 && hi < 0.1, "upper bound stays plausible: {hi}");
		let (lo, hi) = wilson(100, 100, Z95);
		assert!(lo > 0.9 && lo < 1.0, "lower bound below 1: {lo}");
		assert!(hi <= 1.0 && hi > 0.99, "upper bound cannot exceed 1: {hi}");
	}

	#[test]
	fn wilson_brackets_the_point_estimate_and_tightens_with_n() {
		let (lo, hi) = wilson(50, 100, Z95);
		assert!(lo < 0.5 && hi > 0.5, "interval brackets p=0.5");
		let (lo2, hi2) = wilson(500, 1000, Z95);
		assert!(
			(hi2 - lo2) < (hi - lo),
			"10x the sample gives a tighter interval"
		);
	}

	#[test]
	fn mcnemar_calls_an_even_split_insignificant_and_a_lopsided_one_significant() {
		assert_eq!(mcnemar_exact(0, 0), 1.0, "no discordant pairs, no evidence");
		assert!(
			mcnemar_exact(8, 5) > 0.5,
			"the granite-vs-qwen embed split was a tie"
		);
		assert!(
			mcnemar_exact(20, 2) < 0.001,
			"a 20-2 split is strong evidence"
		);
		assert!(
			mcnemar_exact(6, 0) < 0.05,
			"a clean sweep of 6 is significant"
		);
	}

	#[test]
	fn mcnemar_is_symmetric_and_never_exceeds_one() {
		for (a, b) in [(3, 7), (1, 1), (0, 4), (12, 9)] {
			assert!((mcnemar_exact(a, b) - mcnemar_exact(b, a)).abs() < 1e-12);
			assert!(mcnemar_exact(a, b) <= 1.0);
		}
	}
}
