//! The vector-index seam behind [`GraphGnn`](super::graph::GraphGnn)'s entity,
//! GNN, and reason indices.
//!
//! A backend is either [`Resident`](VectorBackend::Resident) — an in-memory
//! [`HnswIndex`], the historical behavior for a resident-sized set — or
//! [`Disk`](VectorBackend::Disk): a memory-mapped Vamana snapshot plus a small
//! in-RAM delta for post-snapshot writes, the path that keeps a huge resident
//! set off the heap (see `docs/superpowers/plans/2026-06-12-diskann-wiring.md`).
//! Every method mirrors the matching [`HnswIndex`] signature, so the
//! `base::search` call sites never learn which backend they hit; the routing
//! decision lives entirely in [`GraphGnn::rebuild_index`].

use std::collections::HashSet;

use super::diskann::DiskIndex;
use super::hnsw::{HnswHit, HnswIndex};
use super::util::cmp_rank;
use crate::quant::QuantizationMode;

/// A vector index the graph searches and mutates.
pub enum VectorBackend {
	/// In-memory HNSW — the index for a resident-sized set, mutated in place.
	Resident(HnswIndex),
	/// A disk-resident Vamana `snapshot` (immutable, memory-mapped) overlaid with
	/// an in-RAM `delta` of post-snapshot writes and a `tombstones` set.
	///
	/// Invariant (so a search never double-counts or serves a stale vector):
	/// - `insert(id, v)` writes `v` to `delta` AND tombstones `id` — the tombstone
	///   shadows any now-stale copy of `id` in the snapshot.
	/// - `delete(id)` removes `id` from `delta` AND tombstones it.
	/// - A search reads `snapshot` MINUS tombstones, unioned with `delta`. Because
	///   every delta id is tombstoned, no id is served from both halves.
	Disk {
		snapshot: DiskIndex,
		delta: HnswIndex,
		tombstones: HashSet<String>,
	},
}

impl VectorBackend {
	/// A fresh resident (in-memory HNSW) backend — the default for a new or
	/// rebuilt index. Mirrors [`HnswIndex::with_mode`].
	pub fn resident(m: usize, ef_construction: usize, quant_mode: QuantizationMode) -> Self {
		Self::Resident(HnswIndex::with_mode(m, ef_construction, quant_mode))
	}

	/// A disk-backed backend over `snapshot`, starting with an empty delta and no
	/// tombstones. The delta uses `quant_mode` so its in-RAM scoring matches the
	/// resident index. (The snapshot is built by
	/// [`GraphGnn::build_entity_disk_index`](super::graph::GraphGnn::build_entity_disk_index).)
	pub fn disk(snapshot: DiskIndex, quant_mode: QuantizationMode) -> Self {
		Self::Disk {
			snapshot,
			delta: HnswIndex::with_mode(16, 200, quant_mode),
			tombstones: HashSet::new(),
		}
	}

	/// Number of live (searchable) vectors. For [`Disk`](Self::Disk) this is the
	/// non-tombstoned snapshot vectors plus the delta — an O(snapshot) count, not
	/// a hot-path call.
	pub fn len(&self) -> usize {
		match self {
			Self::Resident(h) => h.len(),
			Self::Disk {
				snapshot,
				delta,
				tombstones,
			} => {
				let live_snapshot = snapshot
					.ids()
					.iter()
					.filter(|id| !tombstones.contains(*id))
					.count();
				live_snapshot + delta.len()
			}
		}
	}

	/// Number of post-snapshot writes buffered in the disk `delta` (0 when not
	/// disk-backed). Drives the consolidation trigger: a large delta means the
	/// in-RAM overlay has grown and the snapshot should be rebuilt.
	pub fn pending_delta_len(&self) -> usize {
		match self {
			Self::Resident(_) => 0,
			Self::Disk { delta, .. } => delta.len(),
		}
	}

	/// Whether the index holds no vectors at all. For [`Disk`](Self::Disk) this is
	/// the cheap structural check (snapshot and delta both empty); a fully
	/// tombstoned-but-non-empty snapshot still reports non-empty, which only costs
	/// a search that returns nothing — the guard's purpose is preserved.
	pub fn is_empty(&self) -> bool {
		match self {
			Self::Resident(h) => h.is_empty(),
			Self::Disk {
				snapshot, delta, ..
			} => snapshot.is_empty() && delta.is_empty(),
		}
	}

	/// Insert or replace the vector for `id`.
	pub fn insert(&mut self, id: String, vec: Vec<f64>) {
		match self {
			Self::Resident(h) => h.insert(id, vec),
			Self::Disk {
				delta, tombstones, ..
			} => {
				// Tombstone shadows any stale snapshot copy; delta holds the live one.
				tombstones.insert(id.clone());
				delta.insert(id, vec);
			}
		}
	}

	/// Remove `id` from the index (no-op if absent).
	pub fn delete(&mut self, id: &str) {
		match self {
			Self::Resident(h) => h.delete(id),
			Self::Disk {
				delta, tombstones, ..
			} => {
				delta.delete(id);
				tombstones.insert(id.to_string());
			}
		}
	}

	/// Approximate top-`k` nearest neighbours to `vec` (cosine-similarity
	/// [`HnswHit`]s, nearest first). `ef` is the beam width.
	pub fn search(&self, vec: &[f64], k: usize, ef: usize) -> Vec<HnswHit> {
		match self {
			Self::Resident(h) => h.search(vec, k, ef),
			Self::Disk {
				snapshot,
				delta,
				tombstones,
			} => {
				let q32: Vec<f32> = vec.iter().map(|&x| x as f32).collect();
				let snap = snapshot.search_hits_filtered(&q32, k, ef, &|id| !tombstones.contains(id));
				let live = delta.search(vec, k, ef);
				union_rank(snap, live, k)
			}
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
			Self::Disk {
				snapshot,
				delta,
				tombstones,
			} => {
				let q32: Vec<f32> = vec.iter().map(|&x| x as f32).collect();
				let snap =
					snapshot.search_hits_filtered(&q32, k, ef, &|id| keep(id) && !tombstones.contains(id));
				let live = delta.search_filtered(vec, k, ef, keep);
				union_rank(snap, live, k)
			}
		}
	}
}

/// Merge two hit lists into one ranked top-`k`. Ids are deduped (keeping the
/// higher score — a defensive guard; the `Disk` invariant already prevents an id
/// from appearing in both halves), then ordered by score descending with an
/// id-ascending tiebreak so the `truncate(k)` boundary is deterministic — the
/// same convention as `base::search::merge_hits`.
fn union_rank(a: Vec<HnswHit>, b: Vec<HnswHit>, k: usize) -> Vec<HnswHit> {
	use std::collections::hash_map::Entry;
	let mut by_id: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
	for h in a.into_iter().chain(b) {
		match by_id.entry(h.id) {
			Entry::Occupied(mut e) => {
				if h.score > *e.get() {
					e.insert(h.score);
				}
			}
			Entry::Vacant(e) => {
				e.insert(h.score);
			}
		}
	}
	let mut ranked: Vec<HnswHit> = by_id
		.into_iter()
		.map(|(id, score)| HnswHit { id, score })
		.collect();
	ranked.sort_by(|x, y| cmp_rank(x.score, &x.id, y.score, &y.id));
	ranked.truncate(k);
	ranked
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::diskann::{build_and_save, Params};

	// Deterministic, well-separated vectors (distinct per-dim frequencies).
	fn vec_of(i: usize) -> Vec<f64> {
		(0..8)
			.map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin())
			.collect()
	}

	// Returns the index plus the TempDir that backs its mmap'd files; the caller
	// must keep the TempDir alive for the index's lifetime (dropping it cleans up).
	fn snapshot_over(ids: impl Iterator<Item = usize>) -> (DiskIndex, tempfile::TempDir) {
		let items: Vec<(String, Vec<f32>)> = ids
			.map(|i| {
				(
					format!("e{i}"),
					vec_of(i).iter().map(|&x| x as f32).collect(),
				)
			})
			.collect();
		let dir = tempfile::tempdir().unwrap();
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();
		(idx, dir)
	}

	#[test]
	fn disk_backend_finds_an_insert_made_after_the_snapshot() {
		// Snapshot covers e0..e50; a NEW entity e999 is inserted post-snapshot.
		// A query at e999's vector must surface it from the delta.
		let (snap, _tmp) = snapshot_over(0..50);
		let mut be = VectorBackend::disk(snap, QuantizationMode::None);
		be.insert("e999".into(), vec_of(999));
		let hits = be.search(&vec_of(999), 5, 96);
		assert_eq!(
			hits.first().map(|h| h.id.as_str()),
			Some("e999"),
			"post-snapshot insert is found first"
		);
	}

	#[test]
	fn disk_backend_excludes_a_tombstoned_snapshot_id() {
		// e10 is in the snapshot; deleting it must hide it from search even though
		// the immutable snapshot still physically contains it.
		let (snap, _tmp) = snapshot_over(0..50);
		let mut be = VectorBackend::disk(snap, QuantizationMode::None);
		be.delete("e10");
		let hits = be.search(&vec_of(10), 10, 128);
		assert!(
			!hits.iter().any(|h| h.id == "e10"),
			"tombstoned id absent from results: {hits:?}"
		);
	}

	#[test]
	fn disk_union_top_hit_matches_a_single_index_over_the_whole_corpus() {
		// Split a corpus across snapshot (e0..40) and delta (e40..80). The union's
		// nearest hit for an indexed query must equal that query point — i.e. the
		// disk+delta union ranks like one index over the full corpus.
		let (snap, _tmp) = snapshot_over(0..40);
		let mut be = VectorBackend::disk(snap, QuantizationMode::None);
		for i in 40..80 {
			be.insert(format!("e{i}"), vec_of(i));
		}
		// Query a delta point and a snapshot point; each should rank itself first.
		assert_eq!(
			be.search(&vec_of(63), 5, 128).first().map(|h| h.id.clone()),
			Some("e63".into())
		);
		assert_eq!(
			be.search(&vec_of(7), 5, 128).first().map(|h| h.id.clone()),
			Some("e7".into())
		);
	}

	#[test]
	fn disk_len_counts_live_vectors_after_delete_and_insert() {
		// 50 in snapshot, delete one (live snapshot 49), insert one new (delta 1).
		let (snap, _tmp) = snapshot_over(0..50);
		let mut be = VectorBackend::disk(snap, QuantizationMode::None);
		assert_eq!(be.len(), 50, "fresh snapshot len");
		be.delete("e5");
		be.insert("e500".into(), vec_of(500));
		assert_eq!(be.len(), 50, "49 live snapshot + 1 delta");
		assert!(!be.is_empty());
	}
}
