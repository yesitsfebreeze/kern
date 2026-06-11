//! Semantic query-result cache.
//!
//! The retrieval pipeline's wall-clock is dominated by two LLM calls — HyDE
//! query expansion (~21 s on a local CPU model) and answer synthesis (~12 s) —
//! while the graph traversal between them is sub-millisecond (see
//! `kern profile`). Re-running an identical or near-identical query therefore
//! pays ~30 s to reproduce a result that has not changed. This cache keys on the
//! **raw query embedding** (computed once, cheaply, before HyDE) and returns a
//! stored [`QueryResult`] when a sufficiently similar query has already been
//! answered against an unchanged region of the graph.
//!
//! ## Hit condition
//!
//! An entry hits when both hold:
//! 1. `cosine(query, entry.query) >= theta` — semantic match, so paraphrases and
//!    re-asks collapse onto one entry.
//! 2. the graph's [mutation epoch](GraphGnn::mutation_epoch) is unchanged since
//!    the entry was stored — i.e. no write has touched the graph.
//!
//! ## Invalidation
//!
//! Each entry stamps the graph's [mutation epoch](GraphGnn::mutation_epoch) at
//! creation and is valid only while that epoch is unchanged. Any content
//! mutation — an entity placed or edited, a reason added, a kern registered or
//! deregistered — bumps the epoch (every mutation routes through
//! `get_mut`/`register`/`deregister`), so the whole cache is conservatively
//! flushed on the next lookup.
//!
//! A global epoch rather than per-kern dependency tracking is required for
//! soundness, not mere simplicity: HyDE rewrites the query before search, so a
//! cached query's results can come from kerns far from its raw query vector. A
//! new memory landing in a kern the previous run never touched would be
//! invisible to any per-kern dependency set, and the stale result would silently
//! omit it. Invalidating on *any* mutation closes that hole. The cost is that a
//! write flushes the cache; for a memory daemon, recall and ingest are distinct
//! phases, so between writes the cache hits fully.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::base::graph::GraphGnn;
use crate::base::math::cosine;
use crate::retrieval::answer::QueryResult;

// Cache defaults live in base::constants (QUERY_CACHE_DEFAULT_CAP / _THETA) so
// `config` can default to them without a config -> retrieval module cycle.
use crate::base::constants::{QUERY_CACHE_DEFAULT_CAP, QUERY_CACHE_DEFAULT_THETA};

/// One cached query: the raw query embedding, a non-vector key component
/// (`tag`), its result, and the graph mutation epoch at which it was stored.
/// `tag` folds in everything that changes the result but is not captured by the
/// query vector — the retrieval `mode` and any active filters (kind, scheme,
/// time bounds, min-confidence). Two queries with the same embedding but a
/// different `tag` must never share an entry.
struct Entry {
	qvec: Vec<f64>,
	tag: u64,
	result: QueryResult,
	epoch: u64,
	/// Hash of the exact query text that produced this entry. Lets an identical
	/// re-ask hit *before* the query is embedded (see [`QueryCache::lookup_text`]),
	/// skipping the embedding round-trip entirely — the common agent case of
	/// asking the same question twice.
	text_hash: u64,
}

/// Bounded, LRU semantic cache over query results. Not thread-safe on its own —
/// wrap in a `Mutex` when shared (the daemon holds one alongside the graph).
pub struct QueryCache {
	entries: VecDeque<Entry>,
	cap: usize,
	theta: f64,
}

impl QueryCache {
	/// `cap` bounds the number of cached queries (LRU eviction past it). `theta`
	/// is the cosine floor for a semantic hit — high (≈0.97+) keeps distinct
	/// questions from colliding onto one answer.
	pub fn new(cap: usize, theta: f64) -> Self {
		// `lookup`/`lookup_text` scan every entry linearly — O(cap) per call. That is
		// negligible at the default cap (256; sub-µs against the ~30s LLM path this
		// cache guards), but a cap in the thousands would turn every recall into a
		// real linear scan. Before raising cap that far, replace the VecDeque with a
		// hash index on (tag, text_hash) plus a small vector ANN for the semantic
		// path. The assertion is a debug-build tripwire against an accidental runaway
		// cap arriving from config.
		debug_assert!(cap <= 4096, "QueryCache cap {cap} is large — lookup is O(cap); see the note in new()");
		Self { entries: VecDeque::new(), cap: cap.max(1), theta }
	}

	/// A cache with the given cap/theta, wrapped for sharing across the daemon's
	/// request handlers.
	pub fn shared(cap: usize, theta: f64) -> Arc<Mutex<Self>> {
		Arc::new(Mutex::new(Self::new(cap, theta)))
	}

	/// A cache with the default cap/theta. Used by tests and any caller without a
	/// loaded config.
	pub fn default_shared() -> Arc<Mutex<Self>> {
		Self::shared(QUERY_CACHE_DEFAULT_CAP, QUERY_CACHE_DEFAULT_THETA)
	}

	/// Exact-text fast path, checked *before* embedding the query. Returns a cached
	/// result when the identical query text (same `tag`, current epoch) was already
	/// answered — so a verbatim re-ask skips both the embedding round-trip and the
	/// LLM pipeline. `text_hash` is a stable hash of the query text (see
	/// [`hash_text`]). Promotes the hit to most-recently-used.
	pub fn lookup_text(&mut self, g: &GraphGnn, text_hash: u64, tag: u64) -> Option<QueryResult> {
		let epoch = g.mutation_epoch();
		let hit = self.entries.iter().position(|e| {
			e.epoch == epoch && e.tag == tag && e.text_hash == text_hash
		})?;
		let entry = self.entries.remove(hit)?;
		let result = entry.result.clone();
		self.entries.push_front(entry);
		Some(result)
	}

	/// Return a cached result for `(qvec, tag)` if a semantically-close,
	/// same-`tag` entry exists whose stamped epoch still matches the live graph.
	/// A stale-epoch entry is treated as a miss (and stays until LRU-evicted or
	/// overwritten by a fresh insert). Promotes the hit to most-recently-used.
	pub fn lookup(&mut self, g: &GraphGnn, qvec: &[f64], tag: u64) -> Option<QueryResult> {
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

	/// Store `result` for `(qvec, tag)`, stamped with `epoch` — the mutation epoch
	/// captured *when the result was computed* (under the retrieval lock), NOT the
	/// current epoch. If a write landed between retrieval and this insert, the live
	/// epoch is already ahead, so the entry is born stale and the next lookup
	/// misses — which is correct. Empty results are not cached: a query that found
	/// nothing is cheap to re-run and caching it would suppress a later query after
	/// data lands.
	pub fn insert(&mut self, epoch: u64, text_hash: u64, qvec: Vec<f64>, tag: u64, result: QueryResult) {
		if result.entities.is_empty() {
			return;
		}
		self.entries.push_front(Entry { qvec, tag, result, epoch, text_hash });
		while self.entries.len() > self.cap {
			self.entries.pop_back();
		}
	}

	/// Drop every entry. Use when a structural change is too broad to express as
	/// per-kern invalidation (e.g. a bulk reload).
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

/// Stable per-process hash of a query's text, used as the key for the exact-text
/// fast path. Process-local is fine: the cache itself is in-memory and per-daemon.
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
		k.entities.insert(entity_id.into(), Entity { id: entity_id.into(), ..Default::default() });
		g.register(k);
		g
	}

	fn result_with(entity_id: &str, answer: &str) -> QueryResult {
		QueryResult {
			answer: answer.into(),
			entities: vec![ScoredEntity {
				entity: Entity { id: entity_id.into(), ..Default::default() },
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
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "cached answer"));

		let hit = cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).expect("exact query hits");
		assert_eq!(hit.answer, "cached answer");
	}

	#[test]
	fn semantically_close_query_hits() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));

		// Nearly the same direction → cosine well above 0.95.
		let hit = cache.lookup(&g, &[0.99, 0.01, 0.0], TAG);
		assert!(hit.is_some(), "paraphrase-close query hits");
	}

	#[test]
	fn distant_query_misses() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));

		// Orthogonal → cosine 0 < theta.
		assert!(cache.lookup(&g, &[0.0, 1.0, 0.0], TAG).is_none(), "distant query misses");
	}

	#[test]
	fn exact_text_hits_before_embedding() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let th = hash_text("what is kern");
		cache.insert(g.mutation_epoch(), th, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));

		// Same text → hit without ever touching a query vector.
		assert!(cache.lookup_text(&g, th, TAG).is_some(), "verbatim re-ask hits pre-embed");
		// Different text → miss (caller falls through to embed + semantic lookup).
		assert!(cache.lookup_text(&g, hash_text("something else"), TAG).is_none(), "different text misses");
	}

	#[test]
	fn exact_text_invalidated_by_mutation() {
		let mut g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let th = hash_text("what is kern");
		cache.insert(g.mutation_epoch(), th, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));
		assert!(cache.lookup_text(&g, th, TAG).is_some());
		let _ = g.get_mut("k1");
		assert!(cache.lookup_text(&g, th, TAG).is_none(), "exact-text path also honors epoch invalidation");
	}

	#[test]
	fn different_tag_misses() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], 1, result_with("e1", "ans"));
		// Same vector, different tag (e.g. a filtered query) must not collide.
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], 2).is_none(), "tag mismatch misses");
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], 1).is_some(), "same tag hits");
	}

	#[test]
	fn any_mutation_invalidates() {
		let mut g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(), "valid before mutation");

		// A mutation to the touched kern advances the global epoch.
		let _ = g.get_mut("k1");
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(), "stale after mutation");
	}

	#[test]
	fn mutation_to_any_kern_invalidates_soundness() {
		// The soundness guarantee: a mutation to a kern the cached result did NOT
		// touch (e.g. a new memory routed elsewhere) must STILL invalidate, because
		// HyDE expansion means that kern could now match. Global epoch ensures it.
		let mut g = graph_with_entity("k1", "e1");
		let root_id = g.root.id.clone();
		g.register(Kern::new("k2", &root_id));
		let mut cache = QueryCache::new(8, 0.95);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "ans"));
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(), "valid before mutation");

		// Mutate an unrelated kern → still invalidates (no stale result possible).
		let _ = g.get_mut("k2");
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(), "unrelated mutation also invalidates");
	}

	#[test]
	fn empty_result_not_cached() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(8, 0.95);
		let empty = QueryResult { answer: String::new(), entities: Vec::new(), path_chains: Vec::new() };
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, empty);
		assert_eq!(cache.len(), 0, "empty result is not stored");
	}

	#[test]
	fn lru_evicts_oldest() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(2, 0.999);
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "a"));
		cache.insert(g.mutation_epoch(), 0, vec![0.0, 1.0, 0.0], TAG, result_with("e1", "b"));
		cache.insert(g.mutation_epoch(), 0, vec![0.0, 0.0, 1.0], TAG, result_with("e1", "c")); // evicts "a"

		assert_eq!(cache.len(), 2);
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_none(), "oldest evicted");
		assert!(cache.lookup(&g, &[0.0, 0.0, 1.0], TAG).is_some(), "newest retained");
	}

	#[test]
	fn lookup_promotes_an_entry_protecting_it_from_the_next_eviction() {
		let g = graph_with_entity("k1", "e1");
		let mut cache = QueryCache::new(2, 0.999); // theta high: each vec hits only itself
		cache.insert(g.mutation_epoch(), 0, vec![1.0, 0.0, 0.0], TAG, result_with("e1", "a"));
		cache.insert(g.mutation_epoch(), 0, vec![0.0, 1.0, 0.0], TAG, result_with("e1", "b"));

		// Touch the OLDER entry (A) — it should jump to most-recently-used.
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(), "A hits and is promoted");

		// Inserting C evicts the now-oldest, which is B (A was just promoted).
		cache.insert(g.mutation_epoch(), 0, vec![0.0, 0.0, 1.0], TAG, result_with("e1", "c"));

		assert_eq!(cache.len(), 2);
		assert!(cache.lookup(&g, &[1.0, 0.0, 0.0], TAG).is_some(), "promoted A survived eviction");
		assert!(cache.lookup(&g, &[0.0, 1.0, 0.0], TAG).is_none(), "unpromoted B was evicted");
		assert!(cache.lookup(&g, &[0.0, 0.0, 1.0], TAG).is_some(), "newest C present");
	}
}
