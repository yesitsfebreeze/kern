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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::Kern;

	use crate::test_support::entity as ent;
	fn hit(id: &str, score: f64) -> EntityHit {
		EntityHit {
			entity_id: id.into(),
			score,
		}
	}
	fn scored(id: &str, score: f64) -> ScoredEntity {
		ScoredEntity {
			entity: ent(id),
			score,
		}
	}
	fn find<'a>(rs: &'a [ScoredEntity], id: &str) -> Option<&'a ScoredEntity> {
		rs.iter().find(|s| s.entity.id == id)
	}

	#[test]
	fn entity_seen_in_both_sources_outranks_one_seen_once() {
		// `a` is surfaced by both the beam and the seeds; `b` only by the beam,
		// at the same raw score. Log-sum-exp gives `a` a +ln(2) corroboration
		// boost, so it must score strictly higher and sort first.
		let g = GraphGnn::new();
		let beam = vec![scored("a", 0.5), scored("b", 0.5)];
		let seeds = [hit("a", 0.5)];
		let out = merge(&g, &seeds, beam);

		let a = find(&out, "a").expect("a present");
		let b = find(&out, "b").expect("b present");
		assert!(
			a.score > b.score,
			"corroborated a ({}) > lone b ({})",
			a.score,
			b.score
		);
		// finalize(0.5, 0.5) = 0.5 + ln 2; finalize(0.5) = 0.5.
		assert!((a.score - (0.5 + std::f64::consts::LN_2)).abs() < 1e-9);
		assert!((b.score - 0.5).abs() < 1e-9);
		assert_eq!(out[0].entity.id, "a", "higher score sorts first");
	}

	#[test]
	fn seed_absent_from_graph_and_beam_is_silently_skipped() {
		// `ghost` is neither in the beam nor resolvable via find_entity_in_graph
		// (empty graph), so it contributes a score entry but no thought — it must
		// not appear in the results rather than panic or surface a bare id.
		let g = GraphGnn::new();
		let beam = vec![scored("b", 0.5)];
		let seeds = [hit("ghost", 0.9)];
		let out = merge(&g, &seeds, beam);

		assert!(find(&out, "ghost").is_none(), "unresolvable seed dropped");
		assert_eq!(out.len(), 1, "only the beam entity survives");
		assert_eq!(out[0].entity.id, "b");
	}

	#[test]
	fn seed_only_entity_is_pulled_from_the_graph() {
		// A seed not in the beam but present in the graph is resolved via
		// find_entity_in_graph and included (exercises the Some branch).
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		k.entities.insert("c".into(), ent("c"));
		g.kerns.insert("kx".into(), k);

		let out = merge(&g, &[hit("c", 0.7)], Vec::new());
		let c = find(&out, "c").expect("seed resolved from graph");
		assert!((c.score - 0.7).abs() < 1e-9, "single observation unchanged");
	}
}
