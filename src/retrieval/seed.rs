use crate::base::graph::GraphGnn;
use crate::base::lexical::LexicalIndex;
use crate::base::math::cosine;
use crate::base::search::{search_all_unlocked, search_reasons_all_unlocked, EntityHit};
use crate::config::RetrievalConfig;
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	Content,
	Reason,
	Hybrid,
}

impl Mode {
	pub fn parse(s: &str) -> Self {
		match s.to_lowercase().as_str() {
			"content" => Self::Content,
			"reason" => Self::Reason,
			_ => Self::Hybrid,
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct Weights {
	pub content: f64,
	pub reason: f64,
	pub edge: f64,
	pub lexical: f64,
}

impl Weights {
	pub fn for_mode(cfg: &RetrievalConfig, m: Mode) -> Self {
		let w = match m {
			Mode::Content => cfg.weights_content,
			Mode::Reason => cfg.weights_reason,
			Mode::Hybrid => cfg.weights_hybrid,
		};
		Self {
			content: w.content,
			reason: w.reason,
			edge: w.edge,
			lexical: w.lexical,
		}
	}
}

/// Build the dense seed set for retrieval: vector ANN over the query (or
/// reason-vector ANN in [`Mode::Reason`]) merged with query-independent
/// "important" entities (high cosine to the query AND either a Fact or
/// frequently-accessed), then truncated to `max(k, cfg.seed_k)`.
///
/// This is the dense + importance core only. The lexical seed layer
/// ([`seed_lexical`]) and PageRank are blended on top by the caller
/// (`answer::retrieve`) for Hybrid mode, so they are intentionally not threaded
/// through here.
pub fn seed(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f64],
	k: usize,
	mode: Mode,
) -> Vec<EntityHit> {
	let mut hits = match mode {
		Mode::Reason => seed_by_reason(g, query_vec, k),
		_ => search_all_unlocked(g, query_vec, k),
	};
	let important = seed_important(g, cfg, query_vec);
	hits = merge_seeds(hits, important);
	hits.truncate(k.max(cfg.seed_k));
	hits
}

pub fn seed_lexical(lex: &LexicalIndex, query_text: &str, k: usize) -> Vec<EntityHit> {
	lex.search(query_text, k)
		.into_iter()
		.map(|h| EntityHit {
			entity_id: h.entity_id,
			score: h.score as f64,
		})
		.collect()
}

fn seed_by_reason(g: &GraphGnn, query_vec: &[f64], k: usize) -> Vec<EntityHit> {
	let reason_hits = search_reasons_all_unlocked(g, query_vec, k);
	let mut seen = HashMap::new();
	for rh in &reason_hits {
		let reason = g
			.kern_of_reason(&rh.reason_id)
			.and_then(|kid| g.loaded(kid))
			.and_then(|kern| kern.reasons.get(&rh.reason_id));
		if let Some(r) = reason {
			let entry = seen.entry(r.from.clone()).or_insert(0.0_f64);
			if rh.score > *entry {
				*entry = rh.score;
			}
		}
	}
	let mut hits: Vec<EntityHit> = seen.into_iter().map(EntityHit::from).collect();
	hits.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	hits
}

pub fn seed_important(g: &GraphGnn, cfg: &RetrievalConfig, query_vec: &[f64]) -> Vec<EntityHit> {
	let kerns = g.all();
	let min_cos = cfg.important_min_cosine;
	let access_threshold = cfg.important_access_threshold;
	let mut hits: Vec<EntityHit> = kerns
		.par_iter()
		.flat_map_iter(|kern| {
			kern.entities.values().filter_map(|t| {
				if !t.has_vector() {
					return None;
				}
				let dominated = !t.is_fact() && t.access_count.value_i32() < access_threshold;
				if dominated {
					return None;
				}
				let score = cosine(query_vec, &t.vector);
				if score >= min_cos {
					Some(EntityHit {
						entity_id: t.id.clone(),
						score,
					})
				} else {
					None
				}
			})
		})
		.collect();
	hits.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	hits
}

pub fn merge_seeds(a: Vec<EntityHit>, b: Vec<EntityHit>) -> Vec<EntityHit> {
	let scored = crate::base::math::softmax_merge_scores(
		a.into_iter().chain(b).map(|h| (h.entity_id, h.score)),
	);
	let mut out: Vec<EntityHit> = scored.into_iter().map(EntityHit::from).collect();
	out.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, EntityKind, Kern};

	fn ent(id: &str, vector: Vec<f64>, access: u64, fact: bool) -> Entity {
		let mut e = Entity {
			id: id.into(),
			vector,
			kind: if fact { EntityKind::Fact } else { EntityKind::Claim },
			..Default::default()
		};
		if access > 0 {
			e.access_count.increment("t", access);
		}
		e
	}

	fn graph_with(entities: Vec<Entity>) -> GraphGnn {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for e in entities {
			k.entities.insert(e.id.clone(), e);
		}
		g.kerns.insert("kx".into(), k);
		g
	}

	fn cfg() -> RetrievalConfig {
		RetrievalConfig { important_min_cosine: 0.5, important_access_threshold: 5, ..Default::default() }
	}

	#[test]
	fn seed_important_applies_cosine_and_access_gates() {
		let g = graph_with(vec![
			ent("hot", vec![1.0, 0.0], 10, false), // accessed + aligned -> in
			ent("cold", vec![1.0, 0.0], 0, false), // aligned but not accessed/fact -> out
			ent("fact", vec![1.0, 0.0], 0, true),  // a Fact bypasses the access gate -> in
			ent("off", vec![0.0, 1.0], 10, false), // accessed but cosine 0 < 0.5 -> out
		]);
		let hits = seed_important(&g, &cfg(), &[1.0, 0.0]);
		let ids: std::collections::HashSet<&str> = hits.iter().map(|h| h.entity_id.as_str()).collect();
		assert!(ids.contains("hot"), "accessed + aligned is important");
		assert!(ids.contains("fact"), "a Fact is important regardless of access count");
		assert!(!ids.contains("cold"), "low-access non-fact is dominated");
		assert!(!ids.contains("off"), "below the cosine threshold is excluded");
	}

	#[test]
	fn merge_seeds_pools_by_entity_and_sorts_descending() {
		let a = vec![EntityHit { entity_id: "x".into(), score: 0.6 }];
		let b = vec![
			EntityHit { entity_id: "x".into(), score: 0.8 }, // same id -> pooled into one
			EntityHit { entity_id: "y".into(), score: 0.3 },
		];
		let out = merge_seeds(a, b);
		assert_eq!(out.len(), 2, "duplicate id x collapses to a single hit");
		assert_eq!(out[0].entity_id, "x", "the higher-scoring entity sorts first");
		assert!(out[0].score >= out[1].score, "descending by score");
	}
}
