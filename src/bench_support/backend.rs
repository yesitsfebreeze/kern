//! Pluggable vector backend for the Qdrant head-to-head baseline — Phase 1 of
//! `docs/superpowers/specs/2026-06-12-qdrant-baseline-harness-design.md`.
//!
//! A [`VectorBackend`] indexes pre-embedded [`Doc`]s and answers vector queries,
//! so the same corpus + queries can be scored against kern and (later,
//! feature-gated) Qdrant through *identical* `ndcg`/latency code. The embeddings
//! are computed ONCE by the caller and fed to every backend, so any recall gap is
//! the index — not the embedder (the confound this session proved dominates).
//!
//! This module is the abstraction + kern's reference implementation; the Qdrant
//! adapter and the multi-backend `compare` harness are later phases.

use crate::base::graph::GraphGnn;
use crate::base::math::cosine;
use crate::base::search::{search_all_filtered, search_all_unlocked};
use crate::base::types::{Entity, EntityKind, Kern};
use crate::base::util::cmp_rank;

/// A pre-embedded corpus document. `vector` is kern-native `f64`; a future Qdrant
/// adapter converts to `f32` at its own boundary so both index the same values.
#[derive(Debug, Clone)]
pub struct Doc {
	pub id: String,
	pub vector: Vec<f64>,
	pub kind: Option<EntityKind>,
}

/// A ranked result: entity id + similarity score (descending).
#[derive(Debug, Clone)]
pub struct QueryHit {
	pub id: String,
	pub score: f64,
}

/// A vector index that can be A/B'd against kern in the baseline harness. Both
/// `index` and `query` see the same `Doc`s/vectors as every other backend.
pub trait VectorBackend {
	fn name(&self) -> &str;
	fn index(&mut self, docs: &[Doc]);
	/// Top-`k` nearest ids. When `kind_filter` is set, only that kind is returned
	/// (filtered DURING the search, not post-filtered — the fewer-than-k fix).
	fn query(&self, vec: &[f64], k: usize, kind_filter: Option<EntityKind>) -> Vec<QueryHit>;
	/// Vector-payload bytes (a lower bound on RSS) for the memory column.
	fn vector_bytes(&self) -> usize;
}

/// kern's own vector index (HNSW over `entity_idx`) — the reference backend the
/// Qdrant column is measured against.
#[derive(Default)]
pub struct KernBackend {
	graph: GraphGnn,
}

impl KernBackend {
	pub fn new() -> Self {
		Self::default()
	}
}

impl VectorBackend for KernBackend {
	fn name(&self) -> &str {
		"kern"
	}

	fn index(&mut self, docs: &[Doc]) {
		let mut g = GraphGnn::new();
		let mut kern = Kern::new("k", "");
		for d in docs {
			let e = Entity {
				id: d.id.clone(),
				vector: d.vector.clone(),
				score: 0.5,
				kind: d.kind.unwrap_or(EntityKind::Claim),
				..Default::default()
			};
			kern.entities.insert(e.id.clone(), e);
		}
		g.kerns.insert("k".to_string(), kern);
		g.rebuild_index();
		self.graph = g;
	}

	fn query(&self, vec: &[f64], k: usize, kind_filter: Option<EntityKind>) -> Vec<QueryHit> {
		let hits = match kind_filter {
			Some(kind) => {
				let keep = |id: &str| {
					self
						.graph
						.kern_of_entity(id)
						.and_then(|kid| self.graph.kerns.get(kid))
						.and_then(|kn| kn.entities.get(id))
						.is_some_and(|e| e.kind == kind)
				};
				search_all_filtered(&self.graph, vec, k, &keep)
			}
			None => search_all_unlocked(&self.graph, vec, k),
		};
		hits
			.into_iter()
			.map(|h| QueryHit {
				id: h.entity_id,
				score: h.score,
			})
			.collect()
	}

	fn vector_bytes(&self) -> usize {
		crate::bench_support::memory::estimate_memory(&self.graph).f64_vector_bytes
	}
}

/// Exact brute-force vector search (full scan, cosine) — the ground-truth backend.
/// Comparing kern (approximate HNSW) against this measures how much recall kern's
/// ANN gives up versus exact nearest-neighbour, the "keep the DiskANN recall@k
/// edge" check. O(n) per query, so it is a baseline, not a contender.
#[derive(Default)]
pub struct BruteForceBackend {
	docs: Vec<Doc>,
}

impl BruteForceBackend {
	pub fn new() -> Self {
		Self::default()
	}
}

impl VectorBackend for BruteForceBackend {
	fn name(&self) -> &str {
		"brute"
	}

	fn index(&mut self, docs: &[Doc]) {
		self.docs = docs.to_vec();
	}

	fn query(&self, vec: &[f64], k: usize, kind_filter: Option<EntityKind>) -> Vec<QueryHit> {
		let mut hits: Vec<QueryHit> = self
			.docs
			.iter()
			.filter(|d| kind_filter.is_none_or(|kf| d.kind == Some(kf)))
			.filter(|d| !d.vector.is_empty() && d.vector.len() == vec.len())
			.map(|d| QueryHit {
				id: d.id.clone(),
				score: cosine(vec, &d.vector),
			})
			.collect();
		// Same deterministic ranking as the rest of the stack: score desc, id asc.
		hits.sort_by(|a, b| cmp_rank(a.score, &a.id, b.score, &b.id));
		hits.truncate(k);
		hits
	}

	fn vector_bytes(&self) -> usize {
		let dim = self
			.docs
			.iter()
			.find(|d| !d.vector.is_empty())
			.map_or(0, |d| d.vector.len());
		self.docs.iter().filter(|d| !d.vector.is_empty()).count() * dim * std::mem::size_of::<f64>()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn doc(id: &str, v: Vec<f64>, kind: EntityKind) -> Doc {
		Doc {
			id: id.into(),
			vector: v,
			kind: Some(kind),
		}
	}

	#[test]
	fn kern_backend_indexes_and_returns_the_nearest() {
		let mut b = KernBackend::new();
		b.index(&[
			doc("a", vec![1.0, 0.0], EntityKind::Fact),
			doc("b", vec![0.0, 1.0], EntityKind::Claim),
			doc("c", vec![0.9, 0.1], EntityKind::Claim),
		]);
		let hits = b.query(&[1.0, 0.0], 2, None);
		assert_eq!(hits.len(), 2, "returns k hits");
		assert_eq!(hits[0].id, "a", "exact match is nearest: {hits:?}");
		assert!(hits[0].score >= hits[1].score, "ranked by score descending");
		assert!(b.vector_bytes() > 0, "reports a non-zero vector footprint");
	}

	#[test]
	fn brute_force_backend_returns_exact_nearest() {
		let mut b = BruteForceBackend::new();
		b.index(&[
			doc("a", vec![1.0, 0.0], EntityKind::Fact),
			doc("b", vec![0.0, 1.0], EntityKind::Claim),
			doc("c", vec![0.9, 0.1], EntityKind::Claim),
		]);
		let hits = b.query(&[1.0, 0.0], 3, None);
		assert_eq!(hits[0].id, "a", "the identical vector is the exact nearest");
		assert_eq!(hits[1].id, "c", "then the close one");
		assert!(
			hits[0].score >= hits[1].score && hits[1].score >= hits[2].score,
			"sorted desc"
		);
		// kind filter applies to the exact scan too.
		let f = b.query(&[1.0, 0.0], 5, Some(EntityKind::Fact));
		assert!(
			!f.is_empty() && f.iter().all(|h| h.id == "a"),
			"only the Fact: {f:?}"
		);
	}

	#[test]
	fn kern_backend_kind_filter_returns_only_matching() {
		let mut b = KernBackend::new();
		b.index(&[
			doc("fact", vec![1.0, 0.0], EntityKind::Fact),
			doc("claim1", vec![1.0, 0.0], EntityKind::Claim),
			doc("claim2", vec![1.0, 0.0], EntityKind::Claim),
		]);
		// All three are equally near; a Fact filter must surface only the Fact
		// (filtered during traversal, so it is not lost behind the closer claims).
		let hits = b.query(&[1.0, 0.0], 5, Some(EntityKind::Fact));
		assert!(
			!hits.is_empty() && hits.iter().all(|h| h.id == "fact"),
			"only the Fact survives the filter: {hits:?}"
		);
	}
}
