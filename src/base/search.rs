use super::graph::GraphGnn;
use super::types::{Reason, Entity};
use super::util::cmp_partial;

#[derive(Debug, Clone)]
pub struct EntityHit {
	pub entity_id: String,
	pub score: f64,
}

impl From<(String, f64)> for EntityHit {
	fn from((entity_id, score): (String, f64)) -> Self {
		Self { entity_id, score }
	}
}

#[derive(Debug, Clone)]
pub struct ReasonHit {
	pub reason_id: String,
	pub score: f64,
}

pub fn search_all_unlocked(g: &GraphGnn, vec: &[f64], k: usize) -> Vec<EntityHit> {
	if vec.is_empty() {
		return Vec::new();
	}
	let ef = (k * 2).max(64);
	let mut scores = std::collections::HashMap::new();

	if !g.entity_idx.is_empty() {
		for h in g.entity_idx.search(vec, k, ef) {
			scores.insert(h.id.clone(), h.score);
		}
	}

	if !g.gnn_entity_idx.is_empty() {
		for h in g.gnn_entity_idx.search(vec, k, ef) {
			let entry = scores.entry(h.id.clone()).or_insert(0.0);
			if *entry > 0.0 {
				*entry = 0.4 * *entry + 0.6 * h.score;
			} else {
				*entry = h.score;
			}
		}
	}

	if scores.is_empty() {
		return Vec::new();
	}

	let mut ranked: Vec<_> = scores.into_iter().collect();
	ranked.sort_by(|a, b| cmp_partial(&b.1, &a.1));
	ranked.truncate(k);

	ranked.into_iter().map(EntityHit::from).collect()
}

pub fn search_reasons_all_unlocked(g: &GraphGnn, vec: &[f64], k: usize) -> Vec<ReasonHit> {
	if g.reason_idx.is_empty() || vec.is_empty() {
		return Vec::new();
	}
	let ef = (k * 2).max(64);
	g.reason_idx
		.search(vec, k, ef)
		.into_iter()
		.map(|h| ReasonHit {
			reason_id: h.id,
			score: h.score,
		})
		.collect()
}

pub fn find_entity(g: &GraphGnn, id: &str) -> Option<(Entity, String)> {
	if let Some(kid) = g.kern_of_entity(id) {
		if let Some(kern) = g.loaded(kid) {
			if let Some(t) = kern.entities.get(id) {
				return Some((t.clone(), kern.id.clone()));
			}
		}
	}
	for kern in g.all() {
		if let Some(t) = kern.entities.get(id) {
			return Some((t.clone(), kern.id.clone()));
		}
	}
	for kern in g.all() {
		if let Some(r) = kern.refs.get(id) {
			if let Some(ref_kern) = g.loaded(&r.kern_id) {
				if let Some(t) = ref_kern.entities.get(&r.entity_id) {
					return Some((t.clone(), ref_kern.id.clone()));
				}
			}
		}
	}
	None
}

pub fn find_reason(g: &GraphGnn, id: &str) -> Option<(Reason, String)> {
	if let Some(kid) = g.kern_of_reason(id) {
		if let Some(kern) = g.loaded(kid) {
			if let Some(r) = kern.reasons.get(id) {
				return Some((r.clone(), kern.id.clone()));
			}
		}
	}
	for kern in g.all() {
		if let Some(r) = kern.reasons.get(id) {
			return Some((r.clone(), kern.id.clone()));
		}
	}
	None
}
