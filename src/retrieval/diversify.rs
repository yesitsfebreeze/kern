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
		selected.push(pool.remove(best_i));
	}

	*results = selected;
	results.extend(tail);
	results.truncate(cfg.max_deliver_results);
}
