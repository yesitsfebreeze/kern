//! Federated hit fusion: merge the per-peer result lists a gossip query fans
//! out into one ranked list. Two strategies, picked by the caller's trust model:
//!
//! - [`online_softmax_merge_hits`] pools each entity's per-peer scores with
//!   log-sum-exp ([`OnlineSoftmax`]). An entity returned by *several* peers earns
//!   a `+ln(count)` corroboration boost over an otherwise-equal entity seen once
//!   — multi-peer agreement is treated as positive evidence. Preferred over a
//!   plain average, which would *dilute* a strong corroborated hit toward the
//!   mean instead of rewarding the agreement.
//! - [`trimmed_mean_merge_hits`] is the Sybil-resistant alternative: it averages
//!   each entity's per-peer scores after trimming the extremes, so a minority of
//!   peers reporting outlier scores can't swing the result.
//!
//! Both sort by score descending with a deterministic id tie-break, then cap at
//! `top_k`.

use std::collections::HashMap;

use crate::base::math::OnlineSoftmax;
use crate::base::search::EntityHit;
use crate::base::util::cmp_partial;

/// Fuse per-peer hit `lists` by log-sum-exp of each entity's per-peer scores
/// (the corroboration-rewarding strategy — see the module doc), sort by score
/// descending with an ascending-id tie-break for determinism, and cap at
/// `top_k`. An entity seen once is unchanged (`x + ln 1 = x`).
pub fn online_softmax_merge_hits(lists: &[&[EntityHit]], top_k: usize) -> Vec<EntityHit> {
	let mut acc: HashMap<String, OnlineSoftmax> = HashMap::new();
	for list in lists {
		for hit in list.iter() {
			acc
				.entry(hit.entity_id.clone())
				.or_default()
				.update(hit.score);
		}
	}
	let mut out: Vec<EntityHit> = acc
		.into_iter()
		.map(|(id, s)| EntityHit {
			entity_id: id,
			score: s.finalize(),
		})
		.collect();
	out.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
			.then_with(|| a.entity_id.cmp(&b.entity_id))
	});
	if top_k < out.len() {
		out.truncate(top_k);
	}
	out
}

/// Trimmed mean of `values`: drop the lowest and highest `floor(n * trim_pct)`
/// finite samples (each side), then average the middle. `trim_pct` is clamped to
/// `[0, 0.4999]`. Returns `None` for an empty/all-non-finite input or when the
/// trim would remove everything.
pub fn trimmed_mean(values: &[f64], trim_pct: f64) -> Option<f64> {
	if values.is_empty() {
		return None;
	}
	let pct = trim_pct.clamp(0.0, 0.4999);
	let n = values.len();
	let k = ((n as f64) * pct).floor() as usize;
	if 2 * k >= n {
		return None;
	}
	let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
	if sorted.is_empty() {
		return None;
	}
	sorted.sort_by(cmp_partial);
	let m = sorted.len();
	let k = ((m as f64) * pct).floor() as usize;
	if 2 * k >= m {
		return None;
	}
	let slice = &sorted[k..m - k];
	let sum: f64 = slice.iter().sum();
	Some(sum / slice.len() as f64)
}

/// Merge per-peer hit lists by [`trimmed_mean`] of each entity's per-peer scores
/// (falling back to a plain finite mean when the trim leaves nothing), then sort
/// by score descending (ties broken by id) and cap at `top_k`. A Sybil-resistant
/// alternative to [`online_softmax_merge_hits`]: trimming discards a minority of
/// peers reporting outlier scores for the same entity.
pub fn trimmed_mean_merge_hits(
	per_peer: &[&[EntityHit]],
	trim_pct: f64,
	top_k: usize,
) -> Vec<EntityHit> {
	let mut acc: HashMap<String, Vec<f64>> = HashMap::new();
	for list in per_peer {
		for hit in list.iter() {
			acc.entry(hit.entity_id.clone()).or_default().push(hit.score);
		}
	}
	let mut out: Vec<EntityHit> = acc
		.into_iter()
		.filter_map(|(id, scores)| {
			let merged = trimmed_mean(&scores, trim_pct).or_else(|| {
				let finite: Vec<f64> = scores.iter().copied().filter(|v| v.is_finite()).collect();
				if finite.is_empty() {
					None
				} else {
					Some(finite.iter().sum::<f64>() / finite.len() as f64)
				}
			})?;
			Some(EntityHit { entity_id: id, score: merged })
		})
		.collect();
	out.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
			.then_with(|| a.entity_id.cmp(&b.entity_id))
	});
	if top_k < out.len() {
		out.truncate(top_k);
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hit(id: &str, score: f64) -> EntityHit {
		EntityHit { entity_id: id.into(), score }
	}

	#[test]
	fn trimmed_mean_handles_edges_and_trims_extremes() {
		assert_eq!(trimmed_mean(&[], 0.1), None);
		assert_eq!(trimmed_mean(&[5.0], 0.1), Some(5.0));
		// [1,2,3,4,100], trim 0.2 -> k=1 -> drop 1 and 100 -> mean(2,3,4) = 3.
		assert_eq!(trimmed_mean(&[1.0, 2.0, 3.0, 4.0, 100.0], 0.2), Some(3.0));
	}

	#[test]
	fn merge_empty_per_peer_is_empty() {
		assert!(trimmed_mean_merge_hits(&[], 0.1, 10).is_empty());
	}

	#[test]
	fn merge_single_peer_sorts_desc_and_caps_at_top_k() {
		let a = vec![hit("x", 0.2), hit("y", 0.9), hit("z", 0.5)];
		let per: Vec<&[EntityHit]> = vec![&a];
		let out = trimmed_mean_merge_hits(&per, 0.0, 2);
		assert_eq!(out.len(), 2, "capped at top_k");
		assert_eq!(out[0].entity_id, "y");
		assert!((out[0].score - 0.9).abs() < 1e-9);
		assert_eq!(out[1].entity_id, "z");
	}

	#[test]
	fn merge_averages_an_entity_seen_by_two_peers() {
		let a = vec![hit("x", 0.4)];
		let b = vec![hit("x", 0.6)];
		let per: Vec<&[EntityHit]> = vec![&a, &b];
		let out = trimmed_mean_merge_hits(&per, 0.0, 10);
		assert_eq!(out.len(), 1);
		assert!((out[0].score - 0.5).abs() < 1e-9, "trim 0 -> plain mean of 0.4, 0.6");
	}

	// ---- online_softmax_merge_hits ------------------------------------------

	#[test]
	fn softmax_merge_empty_input_is_empty() {
		assert!(online_softmax_merge_hits(&[], 10).is_empty());
		// Lists present but all empty -> still nothing.
		let empty: [EntityHit; 0] = [];
		let lists: Vec<&[EntityHit]> = vec![&empty, &empty];
		assert!(online_softmax_merge_hits(&lists, 10).is_empty());
	}

	#[test]
	fn softmax_merge_single_list_sorts_descending() {
		let a = vec![hit("x", 0.2), hit("y", 0.9), hit("z", 0.5)];
		let lists: Vec<&[EntityHit]> = vec![&a];
		let out = online_softmax_merge_hits(&lists, 10);
		let ids: Vec<&str> = out.iter().map(|h| h.entity_id.as_str()).collect();
		assert_eq!(ids, vec!["y", "z", "x"], "single observation scores unchanged, sorted desc");
	}

	#[test]
	fn softmax_merge_corroboration_boosts_entity_seen_by_two_peers() {
		// `x` is reported by both peers at the same score; `y` by one. Log-sum-exp
		// gives x a +ln(2) boost, so it must outrank y.
		let a = vec![hit("x", 0.5)];
		let b = vec![hit("x", 0.5), hit("y", 0.5)];
		let lists: Vec<&[EntityHit]> = vec![&a, &b];
		let out = online_softmax_merge_hits(&lists, 10);
		assert_eq!(out[0].entity_id, "x", "corroborated hit ranks first");
		assert!((out[0].score - (0.5 + std::f64::consts::LN_2)).abs() < 1e-9);
		assert!((out[1].score - 0.5).abs() < 1e-9, "y seen once is unchanged");
	}

	#[test]
	fn softmax_merge_truncates_to_top_k() {
		let a = vec![hit("x", 0.2), hit("y", 0.9), hit("z", 0.5)];
		let lists: Vec<&[EntityHit]> = vec![&a];
		let out = online_softmax_merge_hits(&lists, 2);
		assert_eq!(out.len(), 2, "capped at top_k");
		assert_eq!(out[0].entity_id, "y");
		assert_eq!(out[1].entity_id, "z");
	}

	#[test]
	fn softmax_merge_tie_break_is_deterministic_by_id() {
		// Equal scores must order by ascending id, stably, regardless of input
		// order — so federated results are reproducible across peers.
		let a = vec![hit("b", 0.5), hit("a", 0.5), hit("c", 0.5)];
		let lists: Vec<&[EntityHit]> = vec![&a];
		let out = online_softmax_merge_hits(&lists, 10);
		let ids: Vec<&str> = out.iter().map(|h| h.entity_id.as_str()).collect();
		assert_eq!(ids, vec!["a", "b", "c"], "ties broken by ascending id");
	}
}
