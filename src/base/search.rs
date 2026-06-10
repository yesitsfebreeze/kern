use super::graph::GraphGnn;
use super::hnsw::HnswHit;
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

/// Merge content-index (`primary`) and GNN-index (`gnn`) hits into a single
/// ranked entity list. A node present in both blends `0.4*content + 0.6*gnn`
/// (the learned re-embedding is trusted more); a node in only one keeps that
/// score. Shared by [`search_all_unlocked`] and [`search_all_filtered`] so the
/// fusion + ranking lives in exactly one place.
fn merge_hits(primary: Vec<HnswHit>, gnn: Vec<HnswHit>, k: usize) -> Vec<EntityHit> {
	let mut scores = std::collections::HashMap::new();
	for h in primary {
		scores.insert(h.id, h.score);
	}
	for h in gnn {
		let entry = scores.entry(h.id).or_insert(0.0);
		if *entry > 0.0 {
			*entry = 0.4 * *entry + 0.6 * h.score;
		} else {
			*entry = h.score;
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

pub fn search_all_unlocked(g: &GraphGnn, vec: &[f64], k: usize) -> Vec<EntityHit> {
	if vec.is_empty() {
		return Vec::new();
	}
	let ef = (k * 2).max(64);
	let primary = if g.entity_idx.is_empty() {
		Vec::new()
	} else {
		g.entity_idx.search(vec, k, ef)
	};
	let gnn = if g.gnn_entity_idx.is_empty() {
		Vec::new()
	} else {
		g.gnn_entity_idx.search(vec, k, ef)
	};
	merge_hits(primary, gnn, k)
}

/// Filtered variant of [`search_all_unlocked`]: only entities whose id passes
/// `keep` are returned, with the filter applied DURING the ANN traversal (see
/// [`crate::base::hnsw::HnswIndex::search_filtered`]). Post-filtering an
/// unfiltered top-k yields fewer than `k` when matches are sparse; this returns
/// a full `k` matching hits. `keep` is built at the retrieval layer from a
/// `QueryOptions` filter (see `score::matches_filter`), keeping this base-layer
/// function free of any retrieval dependency.
pub fn search_all_filtered(
	g: &GraphGnn,
	vec: &[f64],
	k: usize,
	keep: &dyn Fn(&str) -> bool,
) -> Vec<EntityHit> {
	if vec.is_empty() {
		return Vec::new();
	}
	let ef = (k * 2).max(64);
	let primary = if g.entity_idx.is_empty() {
		Vec::new()
	} else {
		g.entity_idx.search_filtered(vec, k, ef, keep)
	};
	let gnn = if g.gnn_entity_idx.is_empty() {
		Vec::new()
	} else {
		g.gnn_entity_idx.search_filtered(vec, k, ef, keep)
	};
	merge_hits(primary, gnn, k)
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

#[cfg(test)]
mod tests {
	use super::*;

	// Build a non-trivial entity index directly (no kerns needed: the filtered
	// search operates on the index + the id predicate).
	fn populated() -> GraphGnn {
		let mut g = GraphGnn::new();
		for i in 0..60 {
			let x = (i as f64 * 0.3).sin();
			let y = (i as f64 * 0.3).cos();
			let z = (i % 5) as f64 * 0.2;
			g.entity_idx.insert(format!("e{i}"), vec![x, y, z]);
		}
		g
	}

	fn even(id: &str) -> bool {
		id.trim_start_matches('e').parse::<usize>().map(|n| n % 2 == 0).unwrap_or(false)
	}

	#[test]
	fn search_all_filtered_returns_only_matching_ids() {
		let g = populated();
		let q = vec![0.0_f64.sin(), 0.0_f64.cos(), 0.0];
		let hits = search_all_filtered(&g, &q, 10, &even);
		assert!(!hits.is_empty(), "filtered search finds matches");
		assert!(
			hits.iter().all(|h| even(&h.entity_id)),
			"every returned id passes the predicate"
		);
	}

	#[test]
	fn search_all_filtered_reject_all_is_empty() {
		let g = populated();
		assert!(search_all_filtered(&g, &[1.0, 0.0, 0.0], 5, &|_| false).is_empty());
	}

	#[test]
	fn unfiltered_equals_filtered_with_always_true() {
		// search_all_filtered with a tautological predicate returns the same id set
		// as the plain search, confirming the filtered path is a faithful superset.
		let g = populated();
		let q = vec![0.5, 0.5, 0.2];
		let plain: std::collections::HashSet<String> =
			search_all_unlocked(&g, &q, 10).into_iter().map(|h| h.entity_id).collect();
		let filt: std::collections::HashSet<String> =
			search_all_filtered(&g, &q, 10, &|_| true).into_iter().map(|h| h.entity_id).collect();
		assert_eq!(plain, filt, "always-true filter == unfiltered search");
	}
}
