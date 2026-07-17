use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::base::graph::GraphGnn;
use crate::base::math::cosine;
use crate::retrieval::answer::QueryResult;

// Defaults live in base::constants so `config` can use them without a config -> retrieval cycle.
use crate::base::constants::{QUERY_CACHE_DEFAULT_CAP, QUERY_CACHE_DEFAULT_THETA};

// `tag` folds mode + filters into the key: same embedding, different `tag` must never share an entry.
struct Entry {
	qvec: Vec<f32>,
	tag: u64,
	result: QueryResult,
	epoch: u64,
	text_hash: u64,
}

pub struct QueryCache {
	entries: VecDeque<Entry>,
	cap: usize,
	theta: f64,
}

impl QueryCache {
	pub fn new(cap: usize, theta: f64) -> Self {
		// lookup/lookup_text scan every entry — O(cap): fine at the default 256, not thousands.
		debug_assert!(
			cap <= 4096,
			"QueryCache cap {cap} is large — lookup is O(cap)"
		);
		Self {
			entries: VecDeque::new(),
			cap: cap.max(1),
			theta,
		}
	}

	pub fn shared(cap: usize, theta: f64) -> Arc<Mutex<Self>> {
		Arc::new(Mutex::new(Self::new(cap, theta)))
	}

	pub fn default_shared() -> Arc<Mutex<Self>> {
		Self::shared(QUERY_CACHE_DEFAULT_CAP, QUERY_CACHE_DEFAULT_THETA)
	}

	pub fn lookup_text(&mut self, g: &GraphGnn, text_hash: u64, tag: u64) -> Option<QueryResult> {
		let epoch = g.mutation_epoch();
		let hit = self
			.entries
			.iter()
			.position(|e| e.epoch == epoch && e.tag == tag && e.text_hash == text_hash)?;
		let entry = self.entries.remove(hit)?;
		let result = entry.result.clone();
		self.entries.push_front(entry);
		Some(result)
	}

	pub fn lookup(&mut self, g: &GraphGnn, qvec: &[f32], tag: u64) -> Option<QueryResult> {
		let epoch = g.mutation_epoch();
		let hit = self.entries.iter().position(|e| {
			e.epoch == epoch
				&& e.tag == tag
				&& e.qvec.len() == qvec.len()
				&& cosine(qvec, &e.qvec) >= self.theta
		})?;
		let entry = self.entries.remove(hit)?;
		let result = entry.result.clone();
		self.entries.push_front(entry);
		Some(result)
	}

	// `epoch` must be captured WHEN the result was computed, not now: a racing write leaves the entry born stale (a miss, not a stale serve).
	pub fn insert(
		&mut self,
		epoch: u64,
		text_hash: u64,
		qvec: Vec<f32>,
		tag: u64,
		result: QueryResult,
	) {
		if result.entities.is_empty() {
			return;
		}
		self.entries.push_front(Entry {
			qvec,
			tag,
			result,
			epoch,
			text_hash,
		});
		while self.entries.len() > self.cap {
			self.entries.pop_back();
		}
	}

	pub fn clear(&mut self) {
		self.entries.clear();
	}

	pub fn len(&self) -> usize {
		self.entries.len()
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}
}

pub fn hash_text(text: &str) -> u64 {
	use std::hash::{Hash, Hasher};
	let mut h = std::collections::hash_map::DefaultHasher::new();
	text.hash(&mut h);
	h.finish()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Kern};
	use crate::retrieval::expand::ScoredEntity;

	fn graph_with_entity(kern_id: &str, entity_id: &str) -> GraphGnn {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let mut k = Kern::new(kern_id, &root_id);
		k.entities.insert(
			entity_id.into(),
			Entity {
				id: entity_id.into(),
				..Default::default()
			},
		);
		g.register(k);
		g
	}

	fn result_with(entity_id: &str, answer: &str) -> QueryResult {
		QueryResult {
			answer: answer.into(),
			entities: vec![ScoredEntity {
				entity: Entity {
					id: entity_id.into(),
					..Default::default()
				},
				score: 1.0,
			}],
			path_chains: Vec::new(),
		}
	}

	const TAG: u64 = 0;

	#[test]
	fn exact_query_hits() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "cached answer"),
		);

		let hit = cache
			.lookup(&g, &[1.0, 0.0, 0.0], TAG)
			.expect("exact query hits");
		assert_eq!(hit.answer, "cached answer");
	}

	#[test]
	fn semantically_close_query_hits() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);

		let hit = cache.lookup(&g, &[0.99, 0.01, 0.0], TAG);
		assert!(hit.is_some(), "paraphrase-close query hits");
	}

	#[test]
	fn distant_query_misses() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);

		assert!(
			cache.lookup(&g, &[0.0, 1.0, 0.0], TAG).is_none(),
			"distant query misses"
		);
	}

	#[test]
	fn exact_text_hits_before_embedding() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let th = hash_text("what is kern");
		cache.insert(
			g.mutation_epoch(),
			th,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);

		assert!(
			cache.lookup_text(&g, th, TAG).is_some(),
			"verbatim re-ask hits pre-embed"
		);
		assert!(
			cache
				.lookup_text(&g, hash_text("something else"), TAG)
				.is_none(),
			"different text misses"
		);
	}

	#[test]
	fn exact_text_invalidated_by_mutation() {
		let mut g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let th = hash_text("what is kern");
		cache.insert(
			g.mutation_epoch(),
			th,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);
		assert!(cache.lookup_text(&g, th, TAG).is_some());
		let _ = g.get_mut("k1");
		assert!(
			cache.lookup_text(&g, th, TAG).is_none(),
			"exact-text path also honors epoch invalidation"
		);
	}

	#[test]
	fn different_tag_misses() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			1,
			result_with("e1", "ans"),
		);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], 2).is_none(),
			"tag mismatch misses"
		);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], 1).is_some(),
			"same tag hits"
		);
	}

	#[test]
	fn any_mutation_invalidates() {
		let mut g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(),
			"valid before mutation"
		);

		let _ = g.get_mut("k1");
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(),
			"stale after mutation"
		);
	}

	#[test]
	fn mutation_to_any_kern_invalidates_soundness() {
		let mut g = graph_with_entity("k1", "e1");
		let root_id = g.root.id.clone();
		g.register(Kern::new("k2", &root_id));
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "ans"),
		);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(),
			"valid before mutation"
		);

		let _ = g.get_mut("k2");
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(),
			"unrelated mutation also invalidates"
		);
	}

	#[test]
	fn empty_result_not_cached() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let empty = QueryResult {
			answer: String::new(),
			entities: Vec::new(),
			path_chains: Vec::new(),
		};
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, empty);
		assert_eq!(cache.len(), 0, "empty result is not stored");
	}

	#[test]
	fn lru_evicts_oldest() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(2, 0.999);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "a"),
		);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![0.0, 1.0, 0.0],
			TAG,
			result_with("e1", "b"),
		);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![0.0, 0.0, 1.0],
			TAG,
			result_with("e1", "c"),
		);

		assert_eq!(cache.len(), 2);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(),
			"oldest evicted"
		);
		assert!(
			cache.lookup(&g, &[0.0, 0.0, 1.0], TAG).is_some(),
			"newest retained"
		);
	}

	#[test]
	fn lookup_promotes_an_entry_protecting_it_from_the_next_eviction() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(2, 0.999);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![1.0, 0.0, 0.0],
			TAG,
			result_with("e1", "a"),
		);
		cache.insert(
			g.mutation_epoch(),
			0,
			vec![0.0, 1.0, 0.0],
			TAG,
			result_with("e1", "b"),
		);

		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(),
			"A hits and is promoted"
		);

		cache.insert(
			g.mutation_epoch(),
			0,
			vec![0.0, 0.0, 1.0],
			TAG,
			result_with("e1", "c"),
		);

		assert_eq!(cache.len(), 2);
		assert!(
			cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(),
			"promoted A survived eviction"
		);
		assert!(
			cache.lookup(&g, &[0.0, 1.0, 0.0], TAG).is_none(),
			"unpromoted B was evicted"
		);
		assert!(
			cache.lookup(&g, &[0.0, 0.0, 1.0], TAG).is_some(),
			"newest C present"
		);
	}
}
