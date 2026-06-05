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

pub fn seed(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f64],
	_query_text: &str,
	k: usize,
	mode: Mode,
	_lexical: Option<&LexicalIndex>,
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
