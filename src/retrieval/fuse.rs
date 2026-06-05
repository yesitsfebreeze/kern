use crate::base::search::EntityHit;
use std::collections::HashMap;

pub fn rrf(lists: &[&[EntityHit]], k_rrf: f64, top_k: usize) -> Vec<EntityHit> {
	let mut agg: HashMap<String, f64> = HashMap::new();
	for list in lists {
		for (i, hit) in list.iter().enumerate() {
			let rank = (i + 1) as f64;
			let contrib = 1.0 / (k_rrf + rank);
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
