use crate::base::graph::GraphGnn;
use crate::base::lexical::LexicalIndex;
use crate::base::math::cosine;
use crate::base::search::{
	search_all_filtered, search_all_unlocked, search_reasons_all_unlocked, EntityHit,
};
use crate::retrieval::score::{matches_filter, QueryOptions};
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
	opts: Option<&QueryOptions>,
) -> Vec<EntityHit> {
	let mut hits = match mode {
		Mode::Reason => seed_by_reason(g, query_vec, k),
		// Filter DURING the ANN traversal when a filter is active: a sparse filter
		// then still yields k matching dense hits, instead of post-filtering an
		// unfiltered top-k down to fewer-than-k (the post-filtering coverage bug).
		// With no active filter this is the unchanged unfiltered path, so unfiltered
		// queries are byte-identical. The `keep` predicate resolves each candidate
		// id to its entity BY REFERENCE (no clone) and reuses `score::matches_filter`
		// — the same predicate the post-filter applies — so the two never diverge.
		_ => match opts {
			Some(o) if o.is_active() => {
				let keep = |id: &str| {
					g.kern_of_entity(id)
						.and_then(|kid| g.kerns.get(kid))
						.and_then(|kern| kern.entities.get(id))
						.is_some_and(|e| matches_filter(e, o))
				};
				search_all_filtered(g, query_vec, k, &keep)
			}
			_ => search_all_unlocked(g, query_vec, k),
		},
	};
	let important = seed_important(g, cfg, query_vec, opts);
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

pub fn seed_important(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f64],
	opts: Option<&QueryOptions>,
) -> Vec<EntityHit> {
	let kerns = g.all();
	let min_cos = cfg.important_min_cosine;
	let access_threshold = cfg.important_access_threshold;
	// When a filter is active, importance must respect it at the SOURCE: otherwise
	// non-matching important entities (Facts / frequently-accessed) crowd the merged
	// seed and truncate matching ones out before the post-filter ever runs. The
	// check is cheap (field comparisons) and runs BEFORE the cosine, so non-matching
	// entities also skip the dot product.
	let active_filter = opts.filter(|o| o.is_active());
	let mut hits: Vec<EntityHit> = kerns
		.par_iter()
		.flat_map_iter(|kern| {
			kern.entities.values().filter_map(|t| {
				if !t.has_vector() {
					return None;
				}
				if let Some(o) = active_filter {
					if !matches_filter(t, o) {
						return None;
					}
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
		let hits = seed_important(&g, &cfg(), &[1.0, 0.0], None);
		let ids: std::collections::HashSet<&str> = hits.iter().map(|h| h.entity_id.as_str()).collect();
		assert!(ids.contains("hot"), "accessed + aligned is important");
		assert!(ids.contains("fact"), "a Fact is important regardless of access count");
		assert!(!ids.contains("cold"), "low-access non-fact is dominated");
		assert!(!ids.contains("off"), "below the cosine threshold is excluded");
	}

	#[test]
	fn seed_important_respects_an_active_filter_at_source() {
		// Two aligned, qualifying important entities — one Fact, one accessed Claim.
		// A kind=Fact filter must drop the Claim at the source (not leave it to be
		// post-filtered), so only the Fact survives. A None filter keeps both.
		let g = graph_with(vec![
			ent("the_fact", vec![1.0, 0.0], 0, true),     // Fact, aligned -> in
			ent("the_claim", vec![1.0, 0.0], 10, false),  // accessed Claim, aligned -> in (unfiltered)
		]);
		let both = seed_important(&g, &cfg(), &[1.0, 0.0], None);
		assert_eq!(both.len(), 2, "no filter keeps both important entities");

		let opts = QueryOptions { kind: Some(EntityKind::Fact), ..Default::default() };
		let facts_only = seed_important(&g, &cfg(), &[1.0, 0.0], Some(&opts));
		let ids: Vec<&str> = facts_only.iter().map(|h| h.entity_id.as_str()).collect();
		assert_eq!(ids, vec!["the_fact"], "kind=Fact filter drops the Claim at the source");
	}

	#[test]
	fn active_kind_filter_seeds_matches_post_filtering_would_miss() {
		// 30 Claims identical to the query (cosine 1.0) bury 3 Facts (cosine ~0.994)
		// below any unfiltered top-k. Importance is disabled (an impossible
		// min_cosine) to isolate the dense seed. Without filtering, the dense top-k
		// is all Claims, so a kind=Fact post-filter would surface ZERO Facts.
		// Filtering during traversal returns the Facts instead — the fewer-than-k fix.
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for i in 0..30 {
			let e = ent(&format!("claim{i}"), vec![1.0, 0.0], 0, false);
			k.entities.insert(e.id.clone(), e);
		}
		for i in 0..3 {
			let e = ent(&format!("fact{i}"), vec![0.9, 0.1], 0, true);
			k.entities.insert(e.id.clone(), e);
		}
		g.kerns.insert("kx".into(), k);
		g.rebuild_index();

		let cfg = RetrievalConfig { important_min_cosine: 1.5, seed_k: 5, ..Default::default() };
		let q = [1.0, 0.0];

		// Unfiltered: the dense top-k is dominated by the closer Claims; no Facts.
		let unfiltered = seed(&g, &cfg, &q, 5, Mode::Content, None);
		assert!(
			unfiltered.iter().all(|h| h.entity_id.starts_with("claim")),
			"unfiltered dense seed is all Claims: {:?}",
			unfiltered.iter().map(|h| &h.entity_id).collect::<Vec<_>>()
		);

		// kind=Fact: filtered traversal surfaces the Facts the post-filter would miss.
		let opts = QueryOptions { kind: Some(EntityKind::Fact), ..Default::default() };
		let filtered = seed(&g, &cfg, &q, 5, Mode::Content, Some(&opts));
		assert!(
			!filtered.is_empty() && filtered.iter().all(|h| h.entity_id.starts_with("fact")),
			"filtered seed returns only matching Facts: {:?}",
			filtered.iter().map(|h| &h.entity_id).collect::<Vec<_>>()
		);
	}

	#[test]
	fn unfiltered_seed_is_unchanged_when_opts_is_inactive() {
		// A None filter and a present-but-empty filter must both take the unfiltered
		// path and return an identical seed (the is_active() gate).
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for i in 0..6 {
			let e = ent(&format!("e{i}"), vec![1.0, i as f64 * 0.01], 0, false);
			k.entities.insert(e.id.clone(), e);
		}
		g.kerns.insert("kx".into(), k);
		g.rebuild_index();
		let cfg = RetrievalConfig { important_min_cosine: 1.5, seed_k: 4, ..Default::default() };
		let q = [1.0, 0.0];

		let none = seed(&g, &cfg, &q, 4, Mode::Content, None);
		let empty = seed(&g, &cfg, &q, 4, Mode::Content, Some(&QueryOptions::default()));
		let ids = |v: &[EntityHit]| v.iter().map(|h| h.entity_id.clone()).collect::<Vec<_>>();
		assert_eq!(ids(&none), ids(&empty), "inactive filter == unfiltered path");
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
