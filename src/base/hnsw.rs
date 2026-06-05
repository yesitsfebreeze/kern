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
