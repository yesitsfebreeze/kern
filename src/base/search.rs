use super::graph::GraphGnn;
use super::hnsw::HnswHit;
use super::types::{Entity, Reason};
use super::util::cmp_rank;

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

/// Weight on the raw content-index score when a node is found in BOTH indices.
const CONTENT_BLEND: f64 = 0.4;
/// Weight on the GNN re-embedding score in the same case. Larger than
/// [`CONTENT_BLEND`] because the learned re-embedding is trusted more; the two
/// sum to 1.0.
const GNN_BLEND: f64 = 0.6;

/// Merge content-index (`primary`) and GNN-index (`gnn`) hits into a single
/// ranked entity list. A node present in both blends
/// `CONTENT_BLEND*content + GNN_BLEND*gnn`; a node in only one keeps that score.
/// Shared by [`search_all_unlocked`] and [`search_all_filtered`] so the fusion +
/// ranking lives in exactly one place.
fn merge_hits(primary: Vec<HnswHit>, gnn: Vec<HnswHit>, k: usize) -> Vec<EntityHit> {
	use std::collections::hash_map::Entry;
	let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
	for h in primary {
		scores.insert(h.id, h.score);
	}
	for h in gnn {
		match scores.entry(h.id) {
			// Present in BOTH the content and GNN indices: blend, trusting the GNN
			// re-embedding more. PRESENCE in the content map — not the score's sign —
			// decides the blend. The `score` is a cosine similarity in [-1, 1]
			// (`1.0 - cosine_distance`), so a content hit with a zero or negative
			// similarity must still blend; the old `*entry > 0.0` proxy silently
			// dropped those content scores by mistaking them for "absent".
			Entry::Occupied(mut e) => {
				let blended = CONTENT_BLEND * *e.get() + GNN_BLEND * h.score;
				e.insert(blended);
			}
			// GNN-only hit: keep its score as-is.
			Entry::Vacant(e) => {
				e.insert(h.score);
			}
		}
	}
	if scores.is_empty() {
		return Vec::new();
	}
	let mut ranked: Vec<_> = scores.into_iter().collect();
	// Score descending, ties broken by id ascending. The id tiebreak makes the
	// order a deterministic total order (the source is a HashMap, whose iteration
	// order is not stable across runs), so which equal-score hits survive the
	// `truncate(k)` boundary is reproducible — same convention as fuse::rrf.
	ranked.sort_by(|a, b| cmp_rank(a.1, &a.0, b.1, &b.0));
	ranked.truncate(k);
	ranked.into_iter().map(EntityHit::from).collect()
}

pub fn search_all_unlocked(g: &GraphGnn, vec: &[f32], k: usize) -> Vec<EntityHit> {
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
	vec: &[f32],
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

pub fn search_reasons_all_unlocked(g: &GraphGnn, vec: &[f32], k: usize) -> Vec<ReasonHit> {
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
			let x = (i as f64 * 0.3).sin() as f32;
			let y = (i as f64 * 0.3).cos() as f32;
			let z = (i % 5) as f32 * 0.2;
			g.entity_idx.insert(format!("e{i}"), vec![x, y, z]);
		}
		g
	}

	fn even(id: &str) -> bool {
		id.trim_start_matches('e')
			.parse::<usize>()
			.map(|n| n % 2 == 0)
			.unwrap_or(false)
	}

	fn hh(id: &str, score: f64) -> HnswHit {
		HnswHit {
			id: id.into(),
			score,
		}
	}

	#[test]
	fn merge_blends_a_nonpositive_content_hit_present_in_both() {
		// Regression: cosine similarity is in [-1, 1], so a content hit can score
		// <= 0 and still be a legitimate top-k result. When the same id also appears
		// in the GNN list it must BLEND, not have its content score discarded. The
		// old `*entry > 0.0` presence proxy overwrote it with the raw GNN score.
		let primary = vec![hh("z", 0.0), hh("n", -0.4)];
		let gnn = vec![hh("z", 0.5), hh("n", 0.5)];
		let out = merge_hits(primary, gnn, 10);
		let score_of = |id: &str| out.iter().find(|h| h.entity_id == id).map(|h| h.score);
		// Blended, NOT overwritten by the raw 0.5 GNN score.
		assert_eq!(
			score_of("z"),
			Some(CONTENT_BLEND * 0.0 + GNN_BLEND * 0.5),
			"zero-sim content still blends"
		);
		assert_eq!(
			score_of("n"),
			Some(CONTENT_BLEND * -0.4 + GNN_BLEND * 0.5),
			"negative-sim content still blends"
		);
	}

	#[test]
	fn merge_keeps_single_index_hits_and_blends_shared_positive() {
		// content-only keeps content score; gnn-only keeps gnn score; shared blends.
		let out = merge_hits(
			vec![hh("c", 0.9), hh("both", 0.8)],
			vec![hh("g", 0.7), hh("both", 0.6)],
			10,
		);
		let score_of = |id: &str| out.iter().find(|h| h.entity_id == id).map(|h| h.score);
		assert_eq!(score_of("c"), Some(0.9), "content-only kept");
		assert_eq!(score_of("g"), Some(0.7), "gnn-only kept");
		assert_eq!(
			score_of("both"),
			Some(CONTENT_BLEND * 0.8 + GNN_BLEND * 0.6),
			"shared blends"
		);
	}

	#[test]
	fn search_all_filtered_returns_only_matching_ids() {
		let g = populated();
		let q = vec![0.0_f32.sin(), 0.0_f32.cos(), 0.0];
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
	fn search_reasons_ranks_by_proximity_and_guards_empty() {
		let mut g = GraphGnn::new();
		g.reason_idx.insert("r_x".into(), vec![1.0, 0.0]);
		g.reason_idx.insert("r_y".into(), vec![0.0, 1.0]);

		let hits = search_reasons_all_unlocked(&g, &[1.0, 0.0], 5);
		assert!(!hits.is_empty(), "reason search returns hits");
		assert_eq!(hits[0].reason_id, "r_x", "closest reason ranks first");
		// Empty index OR empty query -> empty, no panic.
		assert!(search_reasons_all_unlocked(&GraphGnn::new(), &[1.0, 0.0], 5).is_empty());
		assert!(search_reasons_all_unlocked(&g, &[], 5).is_empty());
	}

	#[test]
	fn find_entity_resolves_through_the_ref_indirection_path() {
		use crate::base::types::{Entity, EntityRef, Kern};
		// Entity "real" lives in kern "kb". Kern "ka" only holds a *ref* under a
		// different key ("alias") pointing at it. Looking up "alias" must miss the
		// direct-entity paths and resolve via kern.refs -> ref_kern.entities.
		let mut g = GraphGnn::new();
		let mut kb = Kern::new("kb", "");
		kb.entities.insert(
			"real".into(),
			Entity {
				id: "real".into(),
				..Default::default()
			},
		);
		let mut ka = Kern::new("ka", "");
		ka.refs.insert(
			"alias".into(),
			EntityRef {
				kern_id: "kb".into(),
				entity_id: "real".into(),
			},
		);
		g.kerns.insert("kb".into(), kb);
		g.kerns.insert("ka".into(), ka);

		let (ent, kern_id) = find_entity(&g, "alias").expect("resolved via ref path");
		assert_eq!(ent.id, "real", "ref resolves to the target entity");
		assert_eq!(
			kern_id, "kb",
			"returns the entity's home kern, not the ref's"
		);
		// A bogus id hits none of the three paths -> None.
		assert!(find_entity(&g, "nope").is_none());
	}

	#[test]
	fn unfiltered_equals_filtered_with_always_true() {
		// search_all_filtered with a tautological predicate returns the same id set
		// as the plain search, confirming the filtered path is a faithful superset.
		let g = populated();
		let q = vec![0.5, 0.5, 0.2];
		let plain: std::collections::HashSet<String> = search_all_unlocked(&g, &q, 10)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		let filt: std::collections::HashSet<String> = search_all_filtered(&g, &q, 10, &|_| true)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert_eq!(plain, filt, "always-true filter == unfiltered search");
	}
}
