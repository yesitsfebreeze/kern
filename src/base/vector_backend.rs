//! The vector-index seam behind [`GraphGnn`](super::graph::GraphGnn)'s entity,
//! GNN, and reason indices.
//!
//! Today every backend is [`Resident`](VectorBackend::Resident) — an in-memory
//! [`HnswIndex`], the historical behavior with byte-for-byte identical results.
//! The seam exists so a large resident set can later be served from a
//! disk-resident Vamana (a `Disk { snapshot, delta, tombstones }` variant lands
//! in increment I4 of `docs/superpowers/plans/2026-06-12-diskann-wiring.md`)
//! without disturbing the `base::search` call sites: every method here mirrors
//! the matching [`HnswIndex`] signature, so routing changes stay inside this
//! enum.

use super::hnsw::{HnswHit, HnswIndex};
use crate::quant::QuantizationMode;

/// A vector index that the graph searches and mutates. One variant for now;
/// the disk-resident variant is added when the wiring reaches it.
pub enum VectorBackend {
	/// In-memory HNSW — the index for a resident-sized set, mutated in place.
	Resident(HnswIndex),
}

impl VectorBackend {
	/// A fresh resident (in-memory HNSW) backend — the default for a new or
	/// rebuilt index. Mirrors [`HnswIndex::with_mode`].
	pub fn resident(m: usize, ef_construction: usize, quant_mode: QuantizationMode) -> Self {
		Self::Resident(HnswIndex::with_mode(m, ef_construction, quant_mode))
	}

	/// Number of indexed vectors.
	pub fn len(&self) -> usize {
		match self {
			Self::Resident(h) => h.len(),
		}
	}

	/// Whether the index holds no vectors.
	pub fn is_empty(&self) -> bool {
		match self {
			Self::Resident(h) => h.is_empty(),
		}
	}

	/// Insert or replace the vector for `id`.
	pub fn insert(&mut self, id: String, vec: Vec<f64>) {
		match self {
			Self::Resident(h) => h.insert(id, vec),
		}
	}

	/// Remove `id` from the index (no-op if absent).
	pub fn delete(&mut self, id: &str) {
		match self {
			Self::Resident(h) => h.delete(id),
		}
	}

	/// Approximate top-`k` nearest neighbours to `vec` (cosine-similarity
	/// [`HnswHit`]s, nearest first). `ef` is the beam width.
	pub fn search(&self, vec: &[f64], k: usize, ef: usize) -> Vec<HnswHit> {
		match self {
			Self::Resident(h) => h.search(vec, k, ef),
		}
	}

	/// Filtered top-`k` search: only ids passing `keep` are returned, filtered
	/// during traversal so sparse matches behind non-matches stay reachable.
	pub fn search_filtered(
		&self,
		vec: &[f64],
		k: usize,
		ef: usize,
		keep: &dyn Fn(&str) -> bool,
	) -> Vec<HnswHit> {
		match self {
			Self::Resident(h) => h.search_filtered(vec, k, ef, keep),
		}
	}
}
