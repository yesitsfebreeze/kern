use super::math::cosine_distance;
use super::util::cmp_partial;
use crate::quant::{quantized_cosine_distance, QuantizationMode, QuantizedVec};
use rand::RngExt;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct HnswHit {
	pub id: String,
	pub score: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct AdaptiveEfConfig {
	pub ef_start: usize,
	pub ef_max: usize,
	pub ef_step: usize,
	pub spread_epsilon: f64,
}

impl Default for AdaptiveEfConfig {
	fn default() -> Self {
		Self {
			ef_start: 16,
			ef_max: 128,
			ef_step: 128,
			spread_epsilon: 0.02,
		}
	}
}

struct HnswNode {
	vec: Vec<f64>,
	qvec: Option<QuantizedVec>,
	layers: Vec<Vec<String>>,
}

#[derive(Clone)]
struct Candidate {
	id: String,
	dist: f64,
}

pub struct HnswIndex {
	m: usize,
	m0: usize,
	ef_construction: usize,
	ml: f64,
	nodes: HashMap<String, HnswNode>,
	ep: String,
	max_layer: usize,
	rng: rand::rngs::StdRng,
	quant_mode: QuantizationMode,
}

enum Query<'a> {
	Float(&'a [f64]),
	Int8 { q: QuantizedVec, raw: &'a [f64] },
}

impl<'a> Query<'a> {
	fn new(vec: &'a [f64], mode: QuantizationMode) -> Self {
		match mode {
			QuantizationMode::Int8 => Self::Int8 {
				q: QuantizedVec::encode(vec, QuantizationMode::Int8),
				raw: vec,
			},
			_ => Self::Float(vec),
		}
	}
}

impl HnswIndex {
	pub fn new(m: usize, ef_construction: usize) -> Self {
		Self::with_mode(m, ef_construction, QuantizationMode::None)
	}

	pub fn with_mode(m: usize, ef_construction: usize, quant_mode: QuantizationMode) -> Self {
		use rand::SeedableRng;
		let m = m.max(2);
		Self {
			m,
			m0: m * 2,
			ef_construction,
			ml: 1.0 / (m as f64).ln(),
			nodes: HashMap::new(),
			ep: String::new(),
			max_layer: 0,
			rng: rand::rngs::StdRng::seed_from_u64(42),
			quant_mode,
		}
	}

	pub fn quant_mode(&self) -> QuantizationMode {
		self.quant_mode
	}

	pub fn set_quant_mode(&mut self, mode: QuantizationMode) {
		debug_assert!(self.nodes.is_empty(), "set_quant_mode on a non-empty index");
		self.quant_mode = mode;
	}

	pub fn len(&self) -> usize {
		self.nodes.len()
	}

	pub fn is_empty(&self) -> bool {
		self.nodes.is_empty()
	}

	pub fn delete(&mut self, id: &str) {
		self.nodes.remove(id);
		if self.ep == id {
			self.ep = self.nodes.keys().next().cloned().unwrap_or_default();
		}
	}

	pub fn insert(&mut self, id: String, vec: Vec<f64>) {
		if vec.is_empty() || self.nodes.contains_key(&id) {
			return;
		}
		let level = self.random_level();
		let (stored_vec, qvec) = match self.quant_mode {
			QuantizationMode::Int8 => (
				Vec::new(),
				Some(QuantizedVec::encode(&vec, QuantizationMode::Int8)),
			),
			_ => (vec.clone(), None),
		};
		let node = HnswNode {
			vec: stored_vec,
			qvec,
			layers: vec![Vec::new(); level + 1],
		};
		self.nodes.insert(id.clone(), node);

		if self.ep.is_empty() {
			self.ep = id;
			self.max_layer = level;
			return;
		}

		let query = Query::new(&vec, self.quant_mode);
		let mut ep = self.ep.clone();

		for l in (level + 1..=self.max_layer).rev() {
			ep = self.greedy_nearest(&ep, &query, l);
		}

		let start = level.min(self.max_layer);
		for l in (0..=start).rev() {
			let cap = if l == 0 { self.m0 } else { self.m };
			let candidates = self.beam_search(&ep, &query, l, self.ef_construction);
			let neighbors: Vec<Candidate> = candidates.iter().take(cap).cloned().collect();

			let node = self.nodes.get_mut(&id).expect("node just inserted above");
			while node.layers.len() <= l {
				node.layers.push(Vec::new());
			}
			node.layers[l] = neighbors.iter().map(|n| n.id.clone()).collect();

			for nb in &neighbors {
				let nb_node = match self.nodes.get_mut(&nb.id) {
					Some(n) => n,
					None => continue,
				};
				while nb_node.layers.len() <= l {
					nb_node.layers.push(Vec::new());
				}
				nb_node.layers[l].push(id.clone());
				if nb_node.layers[l].len() > cap {
					let ids: Vec<String> = nb_node.layers[l].clone();
					let pruned = self.prune_neighbors(&nb.id, &ids, cap);
					self
						.nodes
						.get_mut(&nb.id)
						.expect("nb_node fetched via get_mut earlier in loop")
						.layers[l] = pruned;
				}
			}

			if let Some(c) = candidates.first() {
				ep = c.id.clone();
			}
		}

		if level > self.max_layer {
			self.max_layer = level;
			self.ep = id;
		}
	}

	pub fn search(&self, vec: &[f64], k: usize, ef: usize) -> Vec<HnswHit> {
		if self.ep.is_empty() || vec.is_empty() {
			return Vec::new();
		}
		let query = Query::new(vec, self.quant_mode);
		let ef = ef.max(k);
		let mut ep = self.ep.clone();

		for l in (1..=self.max_layer).rev() {
			ep = self.greedy_nearest(&ep, &query, l);
		}

		let candidates = self.beam_search(&ep, &query, 0, ef);
		let k = k.min(candidates.len());
		candidates[..k]
			.iter()
			.map(|c| HnswHit {
				id: c.id.clone(),
				score: 1.0 - c.dist,
			})
			.collect()
	}

	pub fn search_batch(&self, queries: &[&[f64]], k: usize, ef: usize) -> Vec<Vec<HnswHit>> {
		queries.par_iter().map(|q| self.search(q, k, ef)).collect()
	}

	pub fn search_adaptive(&self, vec: &[f64], k: usize, cfg: AdaptiveEfConfig) -> Vec<HnswHit> {
		if self.ep.is_empty() || vec.is_empty() || k == 0 {
			return Vec::new();
		}
		let query = Query::new(vec, self.quant_mode);
		let ef_start = cfg.ef_start.max(k);
		let ef_max = cfg.ef_max.max(ef_start);
		let ef_step = cfg.ef_step.max(1);

		let mut ep = self.ep.clone();
		for l in (1..=self.max_layer).rev() {
			ep = self.greedy_nearest(&ep, &query, l);
		}

		let mut ef = ef_start;
		let mut candidates = self.beam_search(&ep, &query, 0, ef);
		while ef < ef_max && is_ambiguous(&candidates, k, cfg.spread_epsilon) {
			ef = (ef + ef_step).min(ef_max);
			candidates = self.beam_search(&ep, &query, 0, ef);
		}

		let k = k.min(candidates.len());
		candidates[..k]
			.iter()
			.map(|c| HnswHit {
				id: c.id.clone(),
				score: 1.0 - c.dist,
			})
			.collect()
	}

	/// Filtered nearest-neighbour search: return up to `k` hits whose id passes
	/// `keep`, ranked by distance. Unlike post-filtering (search k, then drop the
	/// non-matches — which yields *fewer* than k whenever matches are sparse in
	/// the top-k), this filters DURING traversal: non-matching nodes are still
	/// walked for navigation so matches hidden behind them are reachable, but only
	/// matching nodes enter the result set. This is the filtered-vector-search
	/// guarantee a dedicated vector DB provides.
	///
	/// Cost note: when matches are sparse the frontier stays open longer (the
	/// result set never fills to `ef`), so the worst case approaches a full graph
	/// walk. The `visited` set bounds it to O(nodes).
	pub fn search_filtered(
		&self,
		vec: &[f64],
		k: usize,
		ef: usize,
		keep: &dyn Fn(&str) -> bool,
	) -> Vec<HnswHit> {
		if self.ep.is_empty() || vec.is_empty() || k == 0 {
			return Vec::new();
		}
		let query = Query::new(vec, self.quant_mode);
		let ef = ef.max(k);
		let mut ep = self.ep.clone();
		// Upper layers are pure navigation — no filter, just descend to a good
		// entry point for the filtered beam at layer 0.
		for l in (1..=self.max_layer).rev() {
			ep = self.greedy_nearest(&ep, &query, l);
		}
		let candidates = self.beam_search_filtered(&ep, &query, 0, ef, keep);
		let k = k.min(candidates.len());
		candidates[..k]
			.iter()
			.map(|c| HnswHit {
				id: c.id.clone(),
				score: 1.0 - c.dist,
			})
			.collect()
	}

	/// Beam search whose navigation frontier (`candidates`) includes every visited
	/// node, but whose result set (`results`) admits only nodes passing `keep`.
	/// The frontier keeps expanding while the result set is under `ef` OR a
	/// neighbour is closer than the current worst match, so matches behind walls
	/// of non-matching nodes are still found.
	fn beam_search_filtered(
		&self,
		ep: &str,
		query: &Query<'_>,
		layer: usize,
		ef: usize,
		keep: &dyn Fn(&str) -> bool,
	) -> Vec<Candidate> {
		let ep_dist = self.distance_to_query(ep, query);
		let mut candidates = MinHeap::new();
		let mut results = MaxHeap::new();
		let mut visited = HashSet::new();

		let seed = Candidate {
			id: ep.to_string(),
			dist: ep_dist,
		};
		candidates.push(seed.clone());
		visited.insert(ep.to_string());
		if keep(ep) {
			results.push(seed);
		}

		while let Some(c) = candidates.pop() {
			// If the matching set is full and the nearest frontier node is farther
			// than the worst match, no closer match can exist — stop.
			if results.len() >= ef {
				if let Some(worst) = results.peek() {
					if c.dist > worst.dist {
						break;
					}
				}
			}
			let node = &self.nodes[&c.id];
			if layer >= node.layers.len() {
				continue;
			}
			for nb_id in &node.layers[layer] {
				if !visited.insert(nb_id.clone()) {
					continue;
				}
				if !self.nodes.contains_key(nb_id) {
					continue;
				}
				let d = self.distance_to_query(nb_id, query);
				// Explore (navigate through) this node if the result set isn't full
				// yet, or it could beat the worst match. Non-matching nodes are still
				// pushed to the frontier — that is how we reach matches behind them.
				let worst = results.peek().map(|w| w.dist);
				let explore = results.len() < ef || worst.map_or(true, |w| d < w);
				if explore {
					candidates.push(Candidate {
						id: nb_id.clone(),
						dist: d,
					});
					if keep(nb_id) {
						results.push(Candidate {
							id: nb_id.clone(),
							dist: d,
						});
						if results.len() > ef {
							results.pop();
						}
					}
				}
			}
		}

		let mut out = Vec::with_capacity(results.len());
		while let Some(c) = results.pop() {
			out.push(c);
		}
		out.reverse();
		out
	}

	fn random_level(&mut self) -> usize {
		let r: f64 = self.rng.random::<f64>().max(1e-18);
		let level = (-r.ln() * self.ml).floor() as usize;
		level.min(16)
	}

	fn distance_to_query(&self, node_id: &str, query: &Query<'_>) -> f64 {
		let node = match self.nodes.get(node_id) {
			Some(n) => n,
			None => return 1.0,
		};
		match query {
			Query::Float(v) => cosine_distance(&node.vec, v),
			Query::Int8 { q, raw } => match &node.qvec {
				Some(nq) => quantized_cosine_distance(nq, q),
				None => cosine_distance(&node.vec, raw),
			},
		}
	}

	fn distance_between(&self, a: &str, b: &str) -> f64 {
		let (Some(na), Some(nb)) = (self.nodes.get(a), self.nodes.get(b)) else {
			return 1.0;
		};
		match self.quant_mode {
			QuantizationMode::Int8 => match (&na.qvec, &nb.qvec) {
				(Some(qa), Some(qb)) => quantized_cosine_distance(qa, qb),
				_ => cosine_distance(&na.vec, &nb.vec),
			},
			_ => cosine_distance(&na.vec, &nb.vec),
		}
	}

	fn greedy_nearest(&self, ep: &str, query: &Query<'_>, layer: usize) -> String {
		let mut best = ep.to_string();
		let mut best_dist = self.distance_to_query(ep, query);
		loop {
			let mut changed = false;
			let node = &self.nodes[&best];
			if layer >= node.layers.len() {
				break;
			}
			for nb_id in &node.layers[layer] {
				if self.nodes.contains_key(nb_id) {
					let d = self.distance_to_query(nb_id, query);
					if d < best_dist {
						best_dist = d;
						best = nb_id.clone();
						changed = true;
					}
				}
			}
			if !changed {
				break;
			}
		}
		best
	}

	fn beam_search(&self, ep: &str, query: &Query<'_>, layer: usize, ef: usize) -> Vec<Candidate> {
		let ep_dist = self.distance_to_query(ep, query);
		let mut candidates = MinHeap::new();
		let mut results = MaxHeap::new();
		let mut visited = HashSet::new();

		let seed = Candidate {
			id: ep.to_string(),
			dist: ep_dist,
		};
		candidates.push(seed.clone());
		results.push(seed);
		visited.insert(ep.to_string());

		while let Some(c) = candidates.pop() {
			if results.len() >= ef {
				if let Some(worst) = results.peek() {
					if c.dist > worst.dist {
						break;
					}
				}
			}
			let node = &self.nodes[&c.id];
			if layer >= node.layers.len() {
				continue;
			}
			for nb_id in &node.layers[layer] {
				if !visited.insert(nb_id.clone()) {
					continue;
				}
				if !self.nodes.contains_key(nb_id) {
					continue;
				}
				let d = self.distance_to_query(nb_id, query);
				let dominated = results.len() >= ef && results.peek().is_some_and(|w| d >= w.dist);
				if !dominated {
					let cand = Candidate {
						id: nb_id.clone(),
						dist: d,
					};
					candidates.push(cand.clone());
					results.push(cand);
					if results.len() > ef {
						results.pop();
					}
				}
			}
		}

		let mut out = Vec::with_capacity(results.len());
		while let Some(c) = results.pop() {
			out.push(c);
		}
		out.reverse();
		out
	}

	fn prune_neighbors(&self, center_id: &str, ids: &[String], m: usize) -> Vec<String> {
		let mut pairs: Vec<(String, f64)> = ids
			.iter()
			.filter_map(|id| {
				if self.nodes.contains_key(id) {
					Some((id.clone(), self.distance_between(center_id, id)))
				} else {
					None
				}
			})
			.collect();
		pairs.sort_by(|a, b| cmp_partial(&a.1, &b.1));
		pairs.truncate(m);
		pairs.into_iter().map(|(id, _)| id).collect()
	}
}

fn is_ambiguous(candidates: &[Candidate], k: usize, epsilon: f64) -> bool {
	if candidates.len() < k {
		return true;
	}
	let top = &candidates[..k.min(candidates.len())];
	let Some(best) = top.first() else {
		return false;
	};
	let Some(worst) = top.last() else {
		return false;
	};
	(worst.dist - best.dist) < epsilon
}

struct MinHeap {
	items: Vec<Candidate>,
}

impl MinHeap {
	fn new() -> Self {
		Self { items: Vec::new() }
	}

	fn push(&mut self, c: Candidate) {
		self.items.push(c);
		let mut i = self.items.len() - 1;
		while i > 0 {
			let p = (i - 1) / 2;
			if self.items[i].dist >= self.items[p].dist {
				break;
			}
			self.items.swap(i, p);
			i = p;
		}
	}

	fn pop(&mut self) -> Option<Candidate> {
		if self.items.is_empty() {
			return None;
		}
		let n = self.items.len() - 1;
		self.items.swap(0, n);
		let top = self.items.pop().expect("non-empty checked above");
		let mut i = 0;
		let sz = self.items.len();
		loop {
			let (l, r) = (2 * i + 1, 2 * i + 2);
			let mut s = i;
			if l < sz && self.items[l].dist < self.items[s].dist {
				s = l;
			}
			if r < sz && self.items[r].dist < self.items[s].dist {
				s = r;
			}
			if s == i {
				break;
			}
			self.items.swap(i, s);
			i = s;
		}
		Some(top)
	}
}

struct MaxHeap {
	items: Vec<Candidate>,
}

impl MaxHeap {
	fn new() -> Self {
		Self { items: Vec::new() }
	}

	fn len(&self) -> usize {
		self.items.len()
	}

	fn peek(&self) -> Option<&Candidate> {
		self.items.first()
	}

	fn push(&mut self, c: Candidate) {
		self.items.push(c);
		let mut i = self.items.len() - 1;
		while i > 0 {
			let p = (i - 1) / 2;
			if self.items[i].dist <= self.items[p].dist {
				break;
			}
			self.items.swap(i, p);
			i = p;
		}
	}

	fn pop(&mut self) -> Option<Candidate> {
		if self.items.is_empty() {
			return None;
		}
		let n = self.items.len() - 1;
		self.items.swap(0, n);
		let top = self.items.pop().expect("non-empty checked above");
		let mut i = 0;
		let sz = self.items.len();
		loop {
			let (l, r) = (2 * i + 1, 2 * i + 2);
			let mut s = i;
			if l < sz && self.items[l].dist > self.items[s].dist {
				s = l;
			}
			if r < sz && self.items[r].dist > self.items[s].dist {
				s = r;
			}
			if s == i {
				break;
			}
			self.items.swap(i, s);
			i = s;
		}
		Some(top)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::math::cosine_distance as bf_cosine;
	use crate::base::util::cmp_partial as bf_cmp;
	use rand::{RngExt, SeedableRng};
	use std::collections::HashSet;

	fn rand_vec(rng: &mut rand::rngs::StdRng, dim: usize) -> Vec<f64> {
		(0..dim).map(|_| rng.random::<f64>() * 2.0 - 1.0).collect()
	}

	/// Exact nearest-by-cosine ground truth for the recall assertions.
	fn brute_force_topk(vecs: &[(String, Vec<f64>)], q: &[f64], k: usize) -> HashSet<String> {
		let mut scored: Vec<(String, f64)> = vecs
			.iter()
			.map(|(id, v)| (id.clone(), bf_cosine(v, q)))
			.collect();
		scored.sort_by(|a, b| bf_cmp(&a.1, &b.1));
		scored.into_iter().take(k).map(|(id, _)| id).collect()
	}

	fn random_corpus(seed: u64, n: usize, dim: usize) -> Vec<(String, Vec<f64>)> {
		let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
		(0..n).map(|i| (format!("v{i}"), rand_vec(&mut rng, dim))).collect()
	}

	#[test]
	fn empty_index_returns_nothing() {
		let idx = HnswIndex::new(8, 64);
		assert!(idx.is_empty());
		assert!(idx.search(&[1.0, 0.0], 5, 16).is_empty());
	}

	#[test]
	fn inserts_and_finds_exact_nearest() {
		let mut idx = HnswIndex::new(8, 64);
		idx.insert("x".into(), vec![1.0, 0.0, 0.0]);
		idx.insert("y".into(), vec![0.0, 1.0, 0.0]);
		idx.insert("z".into(), vec![0.0, 0.0, 1.0]);
		let hits = idx.search(&[0.9, 0.1, 0.0], 1, 16);
		assert_eq!(hits[0].id, "x", "nearest by cosine is x");
	}

	#[test]
	fn delete_removes_node_from_results() {
		let mut idx = HnswIndex::new(8, 64);
		idx.insert("x".into(), vec![1.0, 0.0]);
		idx.insert("y".into(), vec![0.0, 1.0]);
		idx.delete("x");
		assert!(idx.search(&[1.0, 0.0], 5, 16).iter().all(|h| h.id != "x"));
	}

	#[test]
	fn recall_matches_brute_force() {
		// The whole point of an ANN index is that its top-k closely tracks the
		// exact top-k. Build a corpus, query it, and require high overlap with the
		// brute-force ground truth. This is the recall number we must beat Qdrant
		// on — without it, the index is unmeasured.
		let dim = 32;
		let corpus = random_corpus(7, 300, dim);
		let mut idx = HnswIndex::new(16, 128);
		for (id, v) in &corpus {
			idx.insert(id.clone(), v.clone());
		}
		let k = 10;
		let queries = 25;
		let mut qrng = rand::rngs::StdRng::seed_from_u64(99);
		let mut total = 0.0;
		for _ in 0..queries {
			let q = rand_vec(&mut qrng, dim);
			let truth = brute_force_topk(&corpus, &q, k);
			let got: HashSet<String> =
				idx.search(&q, k, 128).into_iter().map(|h| h.id).collect();
			total += truth.intersection(&got).count() as f64 / k as f64;
		}
		let recall = total / queries as f64;
		assert!(recall >= 0.85, "HNSW recall@{k} too low: {recall:.3}");
	}

	#[test]
	fn search_filtered_matches_brute_force_over_subset() {
		// Filtered search must equal brute-force ranking restricted to the matching
		// subset — proving it filters DURING traversal (k matches returned), not
		// after (which would drop below k whenever matches are sparse in the raw
		// top-k). Keep is "even-indexed vector id".
		let dim = 16;
		let corpus = random_corpus(21, 240, dim);
		let mut idx = HnswIndex::new(16, 128);
		for (id, v) in &corpus {
			idx.insert(id.clone(), v.clone());
		}
		let keep = |id: &str| {
			id.trim_start_matches('v')
				.parse::<usize>()
				.map(|n| n % 2 == 0)
				.unwrap_or(false)
		};
		let subset: Vec<(String, Vec<f64>)> =
			corpus.iter().filter(|(id, _)| keep(id)).cloned().collect();

		let k = 8;
		let queries = 25;
		let mut qrng = rand::rngs::StdRng::seed_from_u64(55);
		let mut total = 0.0;
		for _ in 0..queries {
			let q = rand_vec(&mut qrng, dim);
			let truth = brute_force_topk(&subset, &q, k);
			let hits = idx.search_filtered(&q, k, 128, &keep);
			assert_eq!(hits.len(), k, "filtered search returned fewer than k matches");
			let got: HashSet<String> = hits.into_iter().map(|h| h.id).collect();
			assert!(
				got.iter().all(|id| keep(id)),
				"filtered search returned a non-matching id"
			);
			total += truth.intersection(&got).count() as f64 / k as f64;
		}
		let recall = total / queries as f64;
		assert!(recall >= 0.85, "filtered recall@{k} too low: {recall:.3}");
	}

	#[test]
	fn search_filtered_reject_all_is_empty() {
		let mut idx = HnswIndex::new(8, 64);
		idx.insert("a".into(), vec![1.0, 0.0]);
		idx.insert("b".into(), vec![0.0, 1.0]);
		assert!(idx.search_filtered(&[1.0, 0.0], 5, 32, &|_| false).is_empty());
	}

	#[test]
	fn search_filtered_finds_single_rare_match() {
		// One matching node among many non-matching ones must still be found —
		// the navigation walks through the non-matches to reach it.
		let dim = 16;
		let corpus = random_corpus(8, 200, dim);
		let mut idx = HnswIndex::new(16, 128);
		for (id, v) in &corpus {
			idx.insert(id.clone(), v.clone());
		}
		let target = "v137";
		let qv = corpus.iter().find(|(id, _)| id == target).map(|(_, v)| v.clone()).unwrap();
		let hits = idx.search_filtered(&qv, 5, 128, &|id| id == target);
		assert_eq!(hits.len(), 1, "the one matching node is found");
		assert_eq!(hits[0].id, target);
	}

	#[test]
	fn int8_recall_tracks_f64() {
		// int8 quantization cuts vector memory 8x (the Qdrant-parity move). It must
		// not wreck retrieval: an int8 index's top-k must closely match the f64
		// index's top-k on the same corpus. Proves the quantized path is usable,
		// not just present.
		let dim = 32;
		let corpus = random_corpus(13, 300, dim);
		let mut f64_idx = HnswIndex::new(16, 128);
		let mut i8_idx = HnswIndex::with_mode(16, 128, QuantizationMode::Int8);
		for (id, v) in &corpus {
			f64_idx.insert(id.clone(), v.clone());
			i8_idx.insert(id.clone(), v.clone());
		}
		let k = 10;
		let queries = 25;
		let mut qrng = rand::rngs::StdRng::seed_from_u64(123);
		let mut total = 0.0;
		for _ in 0..queries {
			let q = rand_vec(&mut qrng, dim);
			let f: HashSet<String> =
				f64_idx.search(&q, k, 128).into_iter().map(|h| h.id).collect();
			let i: HashSet<String> =
				i8_idx.search(&q, k, 128).into_iter().map(|h| h.id).collect();
			total += f.intersection(&i).count() as f64 / k as f64;
		}
		let agreement = total / queries as f64;
		assert!(agreement >= 0.75, "int8 vs f64 top-{k} agreement too low: {agreement:.3}");
	}
}
