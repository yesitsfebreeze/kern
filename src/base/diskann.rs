//! Disk-resident Vamana (DiskANN-style) ANN index: an α-pruned proximity graph
//! persisted as three mmap'd files. NOT yet wired into the live search path.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use serde::{Deserialize, Serialize};

use crate::base::hnsw::HnswHit;

/// Adjacency padding marker: "no neighbour in this slot".
const SENTINEL: u32 = u32::MAX;

fn le_u32(c: &[u8]) -> u32 {
	u32::from_le_bytes([c[0], c[1], c[2], c[3]])
}

#[derive(Debug, Clone, Copy)]
pub struct Params {
	/// Max out-degree of the graph.
	pub r: usize,
	/// Beam width during construction.
	pub build_l: usize,
	/// α for RobustPrune (>1 keeps longer-range edges for better recall).
	pub alpha: f32,
}

impl Default for Params {
	fn default() -> Self {
		Self {
			r: 32,
			build_l: 64,
			alpha: 1.2,
		}
	}
}

#[derive(Serialize, Deserialize)]
struct Meta {
	dim: usize,
	count: usize,
	r: usize,
	entry: u32,
	ids: Vec<String>,
}

fn meta_path(dir: &Path) -> PathBuf {
	dir.join("meta.bin")
}
fn vectors_path(dir: &Path) -> PathBuf {
	dir.join("vectors.bin")
}
fn graph_path(dir: &Path) -> PathBuf {
	dir.join("graph.bin")
}

/// `1 - cos`; mismatched or zero-norm inputs yield the max distance `1.0`.
fn cos_dist(a: &[f32], b: &[f32]) -> f32 {
	if a.len() != b.len() {
		return 1.0;
	}
	let mut dot = 0.0f32;
	let mut na = 0.0f32;
	let mut nb = 0.0f32;
	for i in 0..a.len() {
		dot += a[i] * b[i];
		na += a[i] * a[i];
		nb += b[i] * b[i];
	}
	if na == 0.0 || nb == 0.0 {
		return 1.0;
	}
	1.0 - dot / (na.sqrt() * nb.sqrt())
}

/// Greedy beam search. Returns `(beam, visited)`: `beam` sorted nearest first,
/// `visited` every node expanded (feeds construction's RobustPrune).
fn greedy(
	entry: u32,
	beam_l: usize,
	dist: &mut dyn FnMut(u32) -> f32,
	neighbors: &dyn Fn(u32) -> Vec<u32>,
) -> (Vec<(f32, u32)>, Vec<u32>) {
	let mut beam: Vec<(f32, u32)> = vec![(dist(entry), entry)];
	let mut in_beam: HashSet<u32> = HashSet::from([entry]);
	let mut visited: HashSet<u32> = HashSet::new();

	loop {
		let next = beam
			.iter()
			.filter(|(_, id)| !visited.contains(id))
			.min_by(|a, b| a.0.total_cmp(&b.0))
			.map(|&(_, id)| id);
		let Some(p) = next else { break };
		visited.insert(p);
		for nb in neighbors(p) {
			if in_beam.insert(nb) {
				beam.push((dist(nb), nb));
			}
		}
		beam.sort_by(|a, b| a.0.total_cmp(&b.0));
		if beam.len() > beam_l {
			for (_, id) in beam.drain(beam_l..) {
				in_beam.remove(&id);
			}
		}
	}
	(beam, visited.into_iter().collect())
}

/// RobustPrune: choose ≤ `r` out-neighbours for `p` from `candidates`, dropping
/// any candidate occluded by a closer-to-`p` one under the α test.
fn robust_prune(
	p: u32,
	candidates: &[u32],
	r: usize,
	alpha: f32,
	vec_at: &dyn Fn(u32) -> Vec<f32>,
) -> Vec<u32> {
	let pv = vec_at(p);
	let mut scored: Vec<(f32, u32)> = candidates
		.iter()
		.copied()
		.filter(|&c| c != p)
		.collect::<HashSet<u32>>()
		.into_iter()
		.map(|c| (cos_dist(&pv, &vec_at(c)), c))
		.collect();
	scored.sort_by(|a, b| a.0.total_cmp(&b.0));

	let mut removed = vec![false; scored.len()];
	let mut result: Vec<u32> = Vec::with_capacity(r);
	for i in 0..scored.len() {
		if removed[i] {
			continue;
		}
		if result.len() >= r {
			break;
		}
		let (_, pstar) = scored[i];
		result.push(pstar);
		let pstar_v = vec_at(pstar);
		for j in (i + 1)..scored.len() {
			if removed[j] {
				continue;
			}
			let (dpj, v) = scored[j];
			if alpha * cos_dist(&pstar_v, &vec_at(v)) <= dpj {
				removed[j] = true;
			}
		}
	}
	result
}

/// Build a Vamana graph from `items` and persist it under `dir`. Deterministic
/// (fixed RNG seed) for reproducible indexes. Returns the entity count written.
pub fn build_and_save(
	dir: &Path,
	items: &[(String, Vec<f32>)],
	params: Params,
) -> io::Result<usize> {
	std::fs::create_dir_all(dir)?;
	let count = items.len();
	let dim = items.first().map(|(_, v)| v.len()).unwrap_or(0);
	let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
	let vectors: Vec<Vec<f32>> = items.iter().map(|(_, v)| v.clone()).collect();
	let vec_at = |i: u32| vectors[i as usize].clone();

	let mut adj: Vec<Vec<u32>> = vec![Vec::new(); count];
	let entry = medoid(&vectors);

	if count > 1 {
		use rand::RngExt;
		use rand::SeedableRng;
		let mut rng = rand::rngs::StdRng::seed_from_u64(42);

		// Random R-regular-ish init so the graph is connected before pruning.
		for (i, slot) in adj.iter_mut().enumerate().take(count) {
			let mut nbrs = HashSet::new();
			while nbrs.len() < params.r.min(count - 1) {
				let j = rng.random_range(0..count) as u32;
				if j as usize != i {
					nbrs.insert(j);
				}
			}
			*slot = nbrs.into_iter().collect();
		}

		let mut order: Vec<usize> = (0..count).collect();
		for &alpha in &[1.0f32, params.alpha] {
			for i in (1..count).rev() {
				let j = rng.random_range(0..=i);
				order.swap(i, j);
			}
			for &p in &order {
				let pv = vectors[p].clone();
				// Block scopes the borrow of `adj` so the back-edge updates can mutate it.
				let visited = {
					let mut dist = |i: u32| cos_dist(&pv, &vectors[i as usize]);
					let neighbors = |i: u32| adj[i as usize].clone();
					greedy(entry, params.build_l, &mut dist, &neighbors).1
				};
				let pruned = robust_prune(p as u32, &visited, params.r, alpha, &vec_at);
				adj[p] = pruned.clone();
				for &j in &pruned {
					let ju = j as usize;
					if !adj[ju].contains(&(p as u32)) {
						adj[ju].push(p as u32);
						if adj[ju].len() > params.r {
							let cands = adj[ju].clone();
							adj[ju] = robust_prune(j, &cands, params.r, alpha, &vec_at);
						}
					}
				}
			}
		}
	}

	write_files(dir, dim, count, params.r, entry, &ids, &vectors, &adj)?;
	Ok(count)
}

/// Index node closest to the centroid — a good central entry point.
fn medoid(vectors: &[Vec<f32>]) -> u32 {
	if vectors.is_empty() {
		return 0;
	}
	let dim = vectors[0].len();
	let mut centroid = vec![0.0f32; dim];
	for v in vectors {
		for (c, &x) in centroid.iter_mut().zip(v.iter()) {
			*c += x;
		}
	}
	for c in &mut centroid {
		*c /= vectors.len() as f32;
	}
	let mut best = 0u32;
	let mut best_d = f32::INFINITY;
	for (i, v) in vectors.iter().enumerate() {
		let d = cos_dist(&centroid, v);
		if d < best_d {
			best_d = d;
			best = i as u32;
		}
	}
	best
}

/// Layout under `dir/`: `meta.bin` bincode `Meta`; `vectors.bin` `count×dim` f32
/// LE, fixed stride; `graph.bin` `count×r` u32 LE, `SENTINEL`-padded.
#[allow(clippy::too_many_arguments)] // serializer: grouping the on-disk fields into a struct is churn for no gain
fn write_files(
	dir: &Path,
	dim: usize,
	count: usize,
	r: usize,
	entry: u32,
	ids: &[String],
	vectors: &[Vec<f32>],
	adj: &[Vec<u32>],
) -> io::Result<()> {
	let meta = Meta {
		dim,
		count,
		r,
		entry,
		ids: ids.to_vec(),
	};
	let meta_bytes = bincode::serde::encode_to_vec(&meta, bincode::config::standard())
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
	atomic_write(&meta_path(dir), &meta_bytes)?;

	let mut vbuf = Vec::with_capacity(count * dim * 4);
	for v in vectors {
		for &x in v {
			vbuf.extend_from_slice(&x.to_le_bytes());
		}
	}
	atomic_write(&vectors_path(dir), &vbuf)?;

	let mut gbuf = Vec::with_capacity(count * r * 4);
	for nbrs in adj {
		for slot in 0..r {
			let id = nbrs.get(slot).copied().unwrap_or(SENTINEL);
			gbuf.extend_from_slice(&id.to_le_bytes());
		}
	}
	atomic_write(&graph_path(dir), &gbuf)?;
	Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
	let tmp = path.with_extension("tmp");
	std::fs::write(&tmp, bytes)?;
	std::fs::rename(&tmp, path)
}

/// A memory-mapped, read-only Vamana index opened from disk.
pub struct DiskIndex {
	dim: usize,
	count: usize,
	r: usize,
	entry: u32,
	ids: Vec<String>,
	vectors: Mmap,
	graph: Mmap,
}

impl DiskIndex {
	/// Open an index previously written by [`build_and_save`]. The vector and
	/// graph files are memory-mapped; only `meta` is read into memory.
	pub fn open(dir: &Path) -> io::Result<Self> {
		let corrupt = |msg: &str| io::Error::new(io::ErrorKind::InvalidData, format!("diskann: {msg}"));
		let meta_bytes = std::fs::read(meta_path(dir))?;
		let (meta, _): (Meta, _) =
			bincode::serde::decode_from_slice(&meta_bytes, bincode::config::standard())
				.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
		if meta.ids.len() != meta.count {
			return Err(corrupt("id list length does not match meta count"));
		}
		if meta.count > 0 && meta.entry as usize >= meta.count {
			return Err(corrupt("entry point out of range"));
		}
		let vec_bytes = meta
			.count
			.checked_mul(meta.dim)
			.and_then(|n| n.checked_mul(4))
			.ok_or_else(|| corrupt("meta sizes overflow"))?;
		let graph_bytes = meta
			.count
			.checked_mul(meta.r)
			.and_then(|n| n.checked_mul(4))
			.ok_or_else(|| corrupt("meta sizes overflow"))?;
		let vectors = unsafe { Mmap::map(&std::fs::File::open(vectors_path(dir))?)? };
		let graph = unsafe { Mmap::map(&std::fs::File::open(graph_path(dir))?)? };
		// Validate sizes so a truncated/corrupt index is rejected, not read OOB.
		if vectors.len() != vec_bytes || graph.len() != graph_bytes {
			return Err(corrupt("file size does not match meta"));
		}
		// Every adjacency slot must be SENTINEL or a valid node id; otherwise the
		// beam walk would slice the vector mmap out of bounds mid-search.
		for c in graph.chunks_exact(4) {
			let id = le_u32(c);
			if id != SENTINEL && id as usize >= meta.count {
				return Err(corrupt("graph neighbor id out of range"));
			}
		}
		Ok(Self {
			dim: meta.dim,
			count: meta.count,
			r: meta.r,
			entry: meta.entry,
			ids: meta.ids,
			vectors,
			graph,
		})
	}

	pub fn len(&self) -> usize {
		self.count
	}
	pub fn is_empty(&self) -> bool {
		self.count == 0
	}

	/// The id of every vector in the index, in build order.
	pub fn ids(&self) -> &[String] {
		&self.ids
	}

	fn vec_at(&self, i: u32) -> Vec<f32> {
		let off = i as usize * self.dim * 4;
		self.vectors[off..off + self.dim * 4]
			.chunks_exact(4)
			.map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
			.collect()
	}

	fn neighbors_at(&self, i: u32) -> Vec<u32> {
		let off = i as usize * self.r * 4;
		self.graph[off..off + self.r * 4]
			.chunks_exact(4)
			.map(le_u32)
			.filter(|&id| id != SENTINEL)
			.collect()
	}

	/// Approximate `k` nearest neighbours; `search_l` is the beam width (≥ `k`).
	/// Returns `(id, distance)` nearest first.
	pub fn search(&self, query: &[f32], k: usize, search_l: usize) -> Vec<(String, f32)> {
		if self.count == 0 || k == 0 || query.len() != self.dim {
			return Vec::new();
		}
		let beam_l = search_l.max(k);
		let mut dist = |i: u32| cos_dist(query, &self.vec_at(i));
		let neighbors = |i: u32| self.neighbors_at(i);
		let (mut beam, _) = greedy(self.entry, beam_l, &mut dist, &neighbors);
		beam.truncate(k);
		beam
			.into_iter()
			.map(|(d, i)| (self.ids[i as usize].clone(), d))
			.collect()
	}

	/// [`search`](Self::search) as [`HnswHit`]s with a cosine *similarity* score
	/// (`1.0 - distance`), so disk and in-RAM hits fuse in one ranking.
	pub fn search_hits(&self, query: &[f32], k: usize, search_l: usize) -> Vec<HnswHit> {
		self
			.search(query, k, search_l)
			.into_iter()
			.map(|(id, dist)| HnswHit {
				id,
				score: 1.0 - dist as f64,
			})
			.collect()
	}

	/// [`search_hits`](Self::search_hits) restricted to ids passing `keep`; the
	/// pool widens to `search_l.max(k)` first so a sparse filter still fills `k`.
	pub fn search_hits_filtered(
		&self,
		query: &[f32],
		k: usize,
		search_l: usize,
		keep: &dyn Fn(&str) -> bool,
	) -> Vec<HnswHit> {
		if k == 0 {
			return Vec::new();
		}
		let want = search_l.max(k);
		self
			.search(query, want, want)
			.into_iter()
			.filter(|(id, _)| keep(id))
			.take(k)
			.map(|(id, dist)| HnswHit {
				id,
				score: 1.0 - dist as f64,
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rand_items(n: usize, dim: usize, seed: u64) -> Vec<(String, Vec<f32>)> {
		use rand::RngExt;
		use rand::SeedableRng;
		let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
		(0..n)
			.map(|i| {
				let v: Vec<f32> = (0..dim).map(|_| rng.random::<f32>() - 0.5).collect();
				(format!("e{i}"), v)
			})
			.collect()
	}

	fn brute_topk(items: &[(String, Vec<f32>)], q: &[f32], k: usize) -> Vec<String> {
		let mut scored: Vec<(f32, String)> = items
			.iter()
			.map(|(id, v)| (cos_dist(q, v), id.clone()))
			.collect();
		scored.sort_by(|a, b| a.0.total_cmp(&b.0));
		scored.into_iter().take(k).map(|(_, id)| id).collect()
	}

	#[test]
	fn build_open_search_roundtrip() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(200, 16, 1);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();
		assert_eq!(idx.len(), 200);
		let hits = idx.search(&items[0].1, 5, 64);
		assert_eq!(hits.len(), 5);
		// The query is an indexed point, so it should find itself first.
		assert_eq!(hits[0].0, "e0");
	}

	#[test]
	fn recall_at_10_is_high_vs_brute_force() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(500, 24, 7);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();

		let queries = rand_items(20, 24, 99);
		let mut hit = 0usize;
		let mut total = 0usize;
		for (_, q) in &queries {
			let want: HashSet<String> = brute_topk(&items, q, 10).into_iter().collect();
			let got = idx.search(q, 10, 96);
			for (id, _) in got {
				if want.contains(&id) {
					hit += 1;
				}
			}
			total += want.len();
		}
		let recall = hit as f64 / total as f64;
		assert!(recall >= 0.90, "recall@10 too low: {recall:.3}");
	}

	#[test]
	fn empty_and_single() {
		let dir = tempfile::tempdir().unwrap();
		build_and_save(dir.path(), &[], Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();
		assert!(idx.is_empty());
		assert!(idx.search(&[1.0, 0.0], 5, 16).is_empty());

		let dir2 = tempfile::tempdir().unwrap();
		let one = vec![("solo".to_string(), vec![1.0f32, 0.0, 0.0])];
		build_and_save(dir2.path(), &one, Params::default()).unwrap();
		let idx2 = DiskIndex::open(dir2.path()).unwrap();
		let hits = idx2.search(&[1.0, 0.0, 0.0], 5, 16);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].0, "solo");
	}

	#[test]
	fn search_hits_returns_cosine_similarity_nearest_first() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(200, 16, 1);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();

		let hits = idx.search_hits(&items[0].1, 5, 64);
		assert_eq!(hits.len(), 5);
		assert_eq!(hits[0].id, "e0", "indexed point finds itself first");
		assert!(
			hits[0].score > 0.99,
			"self-similarity ~1.0, got {}",
			hits[0].score
		);
		for w in hits.windows(2) {
			assert!(w[0].score >= w[1].score, "scores must descend: {:?}", hits);
		}
	}

	#[test]
	fn search_hits_filtered_returns_only_matching_and_is_a_subset() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(300, 16, 5);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let idx = DiskIndex::open(dir.path()).unwrap();

		// search_l widened so the sparse even-id filter still yields a full k.
		let even = |id: &str| {
			id.trim_start_matches('e')
				.parse::<usize>()
				.map(|n| n % 2 == 0)
				.unwrap_or(false)
		};
		let q = &items[0].1;
		let filt = idx.search_hits_filtered(q, 10, 128, &even);
		assert!(!filt.is_empty(), "filtered search finds matches");
		assert!(
			filt.iter().all(|h| even(&h.id)),
			"every id passes the predicate"
		);

		let wide: HashSet<String> = idx
			.search_hits(q, 128, 128)
			.into_iter()
			.map(|h| h.id)
			.collect();
		assert!(
			filt.iter().all(|h| wide.contains(&h.id)),
			"filtered hits are drawn from the unfiltered candidate pool"
		);

		assert!(idx.search_hits_filtered(q, 10, 64, &|_| false).is_empty());
		assert!(idx.search_hits_filtered(q, 0, 64, &even).is_empty());
	}

	#[test]
	fn corrupt_index_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(10, 8, 3);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		std::fs::write(vectors_path(dir.path()), b"short").unwrap();
		assert!(DiskIndex::open(dir.path()).is_err());
	}

	#[test]
	fn truncated_graph_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(10, 8, 3);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let full = std::fs::read(graph_path(dir.path())).unwrap();
		std::fs::write(graph_path(dir.path()), &full[..full.len() - 3]).unwrap();
		assert!(DiskIndex::open(dir.path()).is_err());
	}

	#[test]
	fn out_of_range_neighbor_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(10, 8, 3);
		build_and_save(dir.path(), &items, Params::default()).unwrap();
		let mut graph = std::fs::read(graph_path(dir.path())).unwrap();
		// Right size, bogus content: first slot points past the last node.
		graph[..4].copy_from_slice(&(items.len() as u32 + 7).to_le_bytes());
		std::fs::write(graph_path(dir.path()), &graph).unwrap();
		assert!(DiskIndex::open(dir.path()).is_err());
	}

	fn rewrite_meta(dir: &Path, mutate: impl FnOnce(&mut Meta)) {
		let bytes = std::fs::read(meta_path(dir)).unwrap();
		let (mut meta, _): (Meta, _) =
			bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
		mutate(&mut meta);
		let out = bincode::serde::encode_to_vec(&meta, bincode::config::standard()).unwrap();
		std::fs::write(meta_path(dir), out).unwrap();
	}

	#[test]
	fn corrupt_meta_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let items = rand_items(10, 8, 3);
		build_and_save(dir.path(), &items, Params::default()).unwrap();

		rewrite_meta(dir.path(), |m| m.entry = 999);
		assert!(
			DiskIndex::open(dir.path()).is_err(),
			"out-of-range entry point"
		);

		rewrite_meta(dir.path(), |m| {
			m.entry = 0;
			m.ids.pop();
		});
		assert!(
			DiskIndex::open(dir.path()).is_err(),
			"ids shorter than count"
		);
	}
}
