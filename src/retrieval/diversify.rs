use crate::base::math::cosine;
use crate::config::RetrievalConfig;
use crate::retrieval::expand::ScoredEntity;
use std::collections::HashMap;

pub fn dedup_by_section(cfg: &RetrievalConfig, results: &mut Vec<ScoredEntity>) {
	if !cfg.dedup_by_section {
		return;
	}
	let mut best: HashMap<String, usize> = HashMap::new();
	let mut keep: Vec<bool> = vec![true; results.len()];
	for (i, r) in results.iter().enumerate() {
		let section = section_key(r.entity.source.section());
		if section.is_empty() {
			continue;
		}
		match best.get(&section).copied() {
			Some(j) => {
				if results[j].score >= r.score {
					keep[i] = false;
				} else {
					keep[j] = false;
					best.insert(section, i);
				}
			}
			None => {
				best.insert(section, i);
			}
		}
	}
	let mut idx = 0;
	results.retain(|_| {
		let k = keep[idx];
		idx += 1;
		k
	});
}

fn section_key(section: &str) -> String {
	match section.find("#chunk") {
		Some(i) => section[..i].to_string(),
		None => section.to_string(),
	}
}

pub fn mmr(cfg: &RetrievalConfig, query_vec: &[f64], results: &mut Vec<ScoredEntity>) {
	if !cfg.mmr_enabled || results.len() <= cfg.max_deliver_results {
		return;
	}
	let pool_size = cfg.mmr_pool_size.min(results.len());
	if pool_size == 0 {
		return;
	}
	let target = cfg.max_deliver_results.min(pool_size);
	let lambda = cfg.mmr_lambda;

	let tail = results.split_off(pool_size);
	let mut pool: Vec<ScoredEntity> = std::mem::take(results);

	let mut selected: Vec<ScoredEntity> = Vec::with_capacity(target);

	while selected.len() < target && !pool.is_empty() {
		let mut best_i = 0usize;
		let mut best_score = f64::NEG_INFINITY;
		for (i, cand) in pool.iter().enumerate() {
			let sim_q = if !cand.entity.vector.is_empty() && !query_vec.is_empty() {
				cosine(query_vec, &cand.entity.vector)
			} else {
				cand.score
			};
			let max_sim_selected = selected
				.iter()
				.map(|s| {
					if s.entity.vector.is_empty() || cand.entity.vector.is_empty() {
						0.0
					} else {
						cosine(&s.entity.vector, &cand.entity.vector)
					}
				})
				.fold(0.0_f64, f64::max);
			let mmr_val = lambda * sim_q - (1.0 - lambda) * max_sim_selected;
			if mmr_val > best_score {
				best_score = mmr_val;
				best_i = i;
			}
		}
		// swap_remove is O(1) vs remove's O(n) element shift. Safe here because
		// the next iteration re-scans the ENTIRE pool for its max — the post-swap
		// reordering cannot change which candidate is picked next (it only affects
		// the arbitrary resolution of exact MMR-value ties, which MMR doesn't
		// care about). Output order is `selected` push-order, untouched by this.
		selected.push(pool.swap_remove(best_i));
	}

	*results = selected;
	results.extend(tail);
	results.truncate(cfg.max_deliver_results);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Source};

	fn sect(id: &str, section: &str, score: f64) -> ScoredEntity {
		ScoredEntity {
			entity: Entity {
				id: id.into(),
				source: Source::Inline {
					hash: id.into(),
					section: section.into(),
				},
				..Default::default()
			},
			score,
		}
	}

	#[test]
	fn dedup_keeps_highest_per_section() {
		let cfg = RetrievalConfig::default(); // dedup_by_section = true
		let mut results = vec![
			sect("a", "doc#chunk0", 0.4),
			sect("b", "doc#chunk1", 0.9), // same stem "doc" -> higher kept
			sect("c", "other#chunk0", 0.5),
		];
		dedup_by_section(&cfg, &mut results);
		let ids: Vec<&str> = results.iter().map(|r| r.entity.id.as_str()).collect();
		assert!(ids.contains(&"b"), "highest in section kept: {ids:?}");
		assert!(!ids.contains(&"a"), "lower in same section dropped: {ids:?}");
		assert!(ids.contains(&"c"), "distinct section kept: {ids:?}");
		assert_eq!(results.len(), 2);
	}

	#[test]
	fn dedup_keeps_empty_section_entries() {
		let cfg = RetrievalConfig::default();
		let mut results = vec![sect("a", "", 0.1), sect("b", "", 0.2)];
		dedup_by_section(&cfg, &mut results);
		assert_eq!(results.len(), 2, "empty-section entries are never collapsed");
	}

	#[test]
	fn dedup_preserves_relative_order_of_survivors() {
		// `retain` keeps survivors in their original positions — a regression
		// guard so a future rewrite (e.g. to a collect/sort) can't silently
		// reorder the delivered set.
		let cfg = RetrievalConfig::default();
		let mut results = vec![
			sect("first", "alpha#chunk0", 0.5),
			sect("second", "beta#chunk0", 0.9),
			sect("beta_low", "beta#chunk1", 0.2), // same stem as second, lower -> dropped
			sect("third", "gamma#chunk0", 0.7),
		];
		dedup_by_section(&cfg, &mut results);
		let ids: Vec<&str> = results.iter().map(|r| r.entity.id.as_str()).collect();
		assert_eq!(ids, vec!["first", "second", "third"], "survivors keep original order");
	}

	#[test]
	fn dedup_noop_when_disabled() {
		let cfg = RetrievalConfig {
			dedup_by_section: false,
			..Default::default()
		};
		let mut results = vec![sect("a", "doc#chunk0", 0.4), sect("b", "doc#chunk1", 0.9)];
		dedup_by_section(&cfg, &mut results);
		assert_eq!(results.len(), 2, "disabled -> no collapse");
	}

	fn ent(id: &str, vector: Vec<f64>, score: f64) -> ScoredEntity {
		ScoredEntity {
			entity: Entity {
				id: id.into(),
				vector,
				..Default::default()
			},
			score,
		}
	}

	#[test]
	fn mmr_runs_and_selects_diverse_over_near_duplicates() {
		// 26 near-identical vectors + 2 distinct ones. With diversity weighted
		// (lambda 0.3) and a delivery cap of 3, MMR must keep one near-dup and
		// BOTH distinct items — proving it actually runs and diversifies.
		let q = vec![1.0, 0.0, 0.0];
		let mut results: Vec<ScoredEntity> = (0..26)
			.map(|i| ent(&format!("dup{i}"), vec![1.0, 0.0, 0.0], 0.9))
			.collect();
		results.push(ent("distinctB", vec![0.0, 1.0, 0.0], 0.5));
		results.push(ent("distinctC", vec![0.0, 0.0, 1.0], 0.5));

		let cfg = RetrievalConfig {
			mmr_enabled: true,
			mmr_lambda: 0.3,
			mmr_pool_size: 50,
			max_deliver_results: 3,
			..Default::default()
		};
		mmr(&cfg, &q, &mut results);

		assert_eq!(results.len(), 3, "MMR must shrink to max_deliver_results");
		let ids: Vec<&str> = results.iter().map(|r| r.entity.id.as_str()).collect();
		assert!(ids.contains(&"distinctB"), "diverse item B selected: {ids:?}");
		assert!(ids.contains(&"distinctC"), "diverse item C selected: {ids:?}");
		let dups = ids.iter().filter(|id| id.starts_with("dup")).count();
		assert_eq!(dups, 1, "only one near-duplicate should survive: {ids:?}");
	}

	#[test]
	fn mmr_noop_when_disabled() {
		let q = vec![1.0, 0.0];
		let mut results: Vec<ScoredEntity> = (0..30)
			.map(|i| ent(&format!("e{i}"), vec![1.0, 0.0], 0.5))
			.collect();
		let cfg = RetrievalConfig {
			mmr_enabled: false,
			..Default::default()
		};
		mmr(&cfg, &q, &mut results);
		assert_eq!(results.len(), 30, "disabled MMR must not touch results");
	}
}
