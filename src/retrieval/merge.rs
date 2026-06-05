use crate::base::graph::GraphGnn;
use crate::base::math::OnlineSoftmax;
use crate::base::search::EntityHit;
use crate::retrieval::expand::{find_entity_in_graph, ScoredEntity};
use std::collections::HashMap;

/// Fuse the seed list and the expansion beam into one ranked result set.
///
/// Each entity's scores from both sources are pooled with log-sum-exp
/// ([`OnlineSoftmax::finalize`]), so an entity surfaced via *both* paths earns a
/// `+ln(count)` corroboration boost over an otherwise-equal entity seen once.
/// This is intentional: multi-path agreement is positive evidence of relevance.
/// A lone observation is unchanged (`x + ln(1) = x`). The merged value is a
/// relevance magnitude (it may exceed 1.0) that `score::apply_boosts` then
/// scales by confidence and adjusts with additive boosts — it is never treated
/// as a probability. Switch `finalize()` to `running_max()` only if
/// best-score-wins (no corroboration) is explicitly wanted.
pub fn merge(g: &GraphGnn, seeds: &[EntityHit], beam: Vec<ScoredEntity>) -> Vec<ScoredEntity> {
	let mut scores: HashMap<String, OnlineSoftmax> = HashMap::new();
	let mut thoughts: HashMap<String, ScoredEntity> = HashMap::new();

	for st in beam {
		scores
			.entry(st.entity.id.clone())
			.or_default()
			.update(st.score);
		thoughts.entry(st.entity.id.clone()).or_insert(st);
	}

	for s in seeds {
		scores
			.entry(s.entity_id.clone())
			.or_default()
			.update(s.score);
		if !thoughts.contains_key(&s.entity_id) {
			if let Some(t) = find_entity_in_graph(g, &s.entity_id) {
				thoughts.insert(
					s.entity_id.clone(),
					ScoredEntity {
						entity: t,
						score: s.score,
					},
				);
			}
		}
	}

	let mut results: Vec<ScoredEntity> = thoughts
		.into_iter()
		.filter_map(|(id, mut st)| {
			let merged = scores.get(&id)?.finalize();
			st.score = merged;
			Some(st)
		})
		.collect();

	results.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	results
}
