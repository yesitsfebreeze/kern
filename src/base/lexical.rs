use super::graph::GraphGnn;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct LexicalHit {
	pub entity_id: String,
	pub score: f32,
}

#[derive(Default)]
struct Posting {
	tf: u32,
}

struct Inner {
	k1: f32,
	b: f32,
	postings: HashMap<String, HashMap<String, Posting>>,
	doc_len: HashMap<String, u32>,
	total_len: u64,
}

pub struct LexicalIndex {
	inner: RwLock<Inner>,
}

impl LexicalIndex {
	pub fn new_in_ram(k1: f32, b: f32) -> Self {
		Self {
			inner: RwLock::new(Inner {
				k1,
				b,
				postings: HashMap::new(),
				doc_len: HashMap::new(),
				total_len: 0,
			}),
		}
	}

	/// Update the BM25 `k1`/`b` scoring parameters. These are read at QUERY time
	/// (`search_filtered`), so no re-indexing is needed. Wired from
	/// `config.retrieval.bm25_k1`/`bm25_b` at daemon load — without this the
	/// configured values were dead and BM25 always used the hardcoded construction
	/// defaults. Guards against nonsensical inputs (negative `k1`, `b` outside
	/// `[0,1]`, NaN) that would corrupt the score formula.
	pub fn set_bm25_params(&self, k1: f32, b: f32) {
		let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
		if k1.is_finite() {
			inner.k1 = k1.max(0.0);
		}
		if b.is_finite() {
			inner.b = b.clamp(0.0, 1.0);
		}
	}

	pub fn insert(&self, entity_id: &str, text: &str) {
		let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
		inner_insert(&mut inner, entity_id, text);
	}

	pub fn remove(&self, entity_id: &str) {
		let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
		inner_remove(&mut inner, entity_id);
	}

	pub fn search(&self, query: &str, k: usize) -> Vec<LexicalHit> {
		self.search_filtered(query, k, &|_| true)
	}

	/// Like [`search`](Self::search) but drops hits whose entity id fails `keep`
	/// BEFORE the top-`k` truncation. BM25 scores every doc containing a query
	/// token, so filtering pre-truncation returns a full `k` *matching* hits — no
	/// over-fetch, and none of the post-filtering fewer-than-k loss. `keep` is built
	/// at the retrieval layer from a `QueryOptions` filter (`score::matches_filter`),
	/// keeping this base-layer index free of any retrieval dependency.
	pub fn search_filtered(&self, query: &str, k: usize, keep: &dyn Fn(&str) -> bool) -> Vec<LexicalHit> {
		let tokens = tokenize(query);
		if tokens.is_empty() || k == 0 {
			return Vec::new();
		}
		let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
		let n_docs = inner.doc_len.len() as f32;
		if n_docs <= 0.0 {
			return Vec::new();
		}
		let avgdl = (inner.total_len as f32 / n_docs).max(1.0);
		let k1 = inner.k1;
		let b = inner.b;

		let mut scores: HashMap<String, f32> = HashMap::new();
		for tok in &tokens {
			let postings = match inner.postings.get(tok) {
				Some(p) => p,
				None => continue,
			};
			let df = postings.len() as f32;
			let idf = ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln();
			for (doc_id, post) in postings {
				let dl = *inner.doc_len.get(doc_id).unwrap_or(&0) as f32;
				let tf = post.tf as f32;
				let denom = tf + k1 * (1.0 - b + b * dl / avgdl);
				let s = idf * (tf * (k1 + 1.0)) / denom;
				*scores.entry(doc_id.clone()).or_insert(0.0) += s;
			}
		}
		let mut hits: Vec<LexicalHit> = scores
			.into_iter()
			.map(|(id, s)| LexicalHit {
				entity_id: id,
				score: s,
			})
			.collect();
		// Score descending, ties broken by entity_id ascending so the set that
		// survives `truncate(k)` is reproducible across runs (the source is a
		// HashMap with unstable iteration order) — same convention as fuse::rrf.
		hits.retain(|h| keep(&h.entity_id));
		hits.sort_by(|a, b| crate::base::util::cmp_rank(a.score, &a.entity_id, b.score, &b.entity_id));
		hits.truncate(k);
		hits
	}

	pub fn rebuild_from_graph(&self, g: &GraphGnn) {
		// Single write acquisition for the whole rebuild: clear, then insert every
		// entity under the SAME guard. The previous version dropped the lock after
		// clearing and re-acquired it once per entity via self.insert(), which on a
		// large graph meant thousands of lock round-trips (and a window where the
		// index was visibly empty to concurrent readers).
		let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
		inner.postings.clear();
		inner.doc_len.clear();
		inner.total_len = 0;
		for kern in g.all() {
			for t in kern.entities.values() {
				let joined = t.statements.join(" ");
				if !joined.is_empty() {
					inner_insert(&mut inner, &t.id, &joined);
				}
			}
		}
	}

	pub fn doc_count(&self) -> usize {
		self
			.inner
			.read()
			.unwrap_or_else(|e| e.into_inner())
			.doc_len
			.len()
	}
}

/// Tokenize `text`, then upsert `entity_id`'s postings under an already-held
/// write guard (no locking). Removing any prior version first makes it an
/// idempotent upsert. Shared by `insert` (locks, single doc) and
/// `rebuild_from_graph` (one lock, every doc).
fn inner_insert(inner: &mut Inner, entity_id: &str, text: &str) {
	let tokens = tokenize(text);
	inner_remove(inner, entity_id);
	let dl = tokens.len() as u32;
	if dl == 0 {
		return;
	}
	let mut tfs: HashMap<String, u32> = HashMap::new();
	for tok in tokens {
		*tfs.entry(tok).or_insert(0) += 1;
	}
	for (tok, tf) in tfs {
		inner
			.postings
			.entry(tok)
			.or_default()
			.insert(entity_id.to_string(), Posting { tf });
	}
	inner.doc_len.insert(entity_id.to_string(), dl);
	inner.total_len += dl as u64;
}

fn inner_remove(inner: &mut Inner, entity_id: &str) {
	if let Some(dl) = inner.doc_len.remove(entity_id) {
		inner.total_len = inner.total_len.saturating_sub(dl as u64);
	} else {
		return;
	}
	let mut empty: Vec<String> = Vec::new();
	for (tok, postings) in inner.postings.iter_mut() {
		postings.remove(entity_id);
		if postings.is_empty() {
			empty.push(tok.clone());
		}
	}
	for tok in empty {
		inner.postings.remove(&tok);
	}
}

/// Split `text` into lowercased, stemmed terms on any non-alphanumeric
/// boundary. There is no stopword list — common words still index (BM25's idf
/// already down-weights them), so a query for a rare term isn't diluted.
fn tokenize(text: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut cur = String::new();
	for ch in text.chars() {
		if ch.is_alphanumeric() {
			for lc in ch.to_lowercase() {
				cur.push(lc);
			}
		} else if !cur.is_empty() {
			out.push(stem(&cur));
			cur.clear();
		}
	}
	if !cur.is_empty() {
		out.push(stem(&cur));
	}
	out
}

/// Naive suffix-stripping stemmer: strip the FIRST matching suffix from a fixed
/// ordered list, and only when the remaining stem stays longer than 2 chars.
///
/// Deliberately crude, with known failure modes: no irregular-form handling, so
/// "mice"/"ran"/"better" are left as-is and never match their singular/base; and
/// first-match ordering can over-strip ("ties" -> "t" via `ies`). It exists only
/// to collapse the common regular English inflections (plurals, -ing/-ed/-ly)
/// enough to lift lexical recall; a precise Porter/Snowball stemmer would add a
/// dependency for marginal gain at this layer. There is also no stopword removal
/// (see `tokenize`).
fn stem(t: &str) -> String {
	let s = t;
	for suf in &["ing", "edly", "ed", "ly", "ies", "es", "s"] {
		if s.len() > suf.len() + 2 && s.ends_with(suf) {
			return s[..s.len() - suf.len()].to_string();
		}
	}
	s.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Kern};

	#[test]
	fn stem_strips_known_suffixes_and_guards_short_words() {
		assert_eq!(stem("running"), "runn", "`ing` stripped");
		assert_eq!(stem("cats"), "cat", "`s` stripped");
		assert_eq!(stem("happily"), "happi", "`ly` stripped");
		// Too short after stripping (stem must stay > 2 chars) — left intact.
		assert_eq!(stem("bus"), "bus");
		assert_eq!(stem("the"), "the", "no matching suffix");
	}

	#[test]
	fn tokenize_splits_lowercases_and_stems() {
		assert_eq!(tokenize("Running, the Cats!"), vec!["runn", "the", "cat"]);
		assert!(tokenize("   ,.!").is_empty(), "punctuation-only yields no tokens");
	}

	#[test]
	fn set_bm25_params_changes_query_scores_and_clamps_invalid() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.insert("short", "alpha beta");
		idx.insert("long", "alpha alpha alpha gamma delta epsilon");

		let base: Vec<f32> = idx.search("alpha", 10).into_iter().map(|h| h.score).collect();
		// k1/b are query-time params: changing them re-scores with no re-indexing.
		idx.set_bm25_params(2.5, 0.0); // drop length normalisation, raise tf saturation
		let tuned: Vec<f32> = idx.search("alpha", 10).into_iter().map(|h| h.score).collect();
		assert_ne!(base, tuned, "new k1/b change BM25 scores without re-indexing");

		// Nonsensical inputs are clamped, not applied raw (would corrupt the formula).
		idx.set_bm25_params(-5.0, 9.0);
		let hits = idx.search("alpha", 10);
		assert!(!hits.is_empty() && hits.iter().all(|h| h.score.is_finite()), "clamped params keep scores finite: {hits:?}");
		// NaN is ignored entirely (previous valid values retained → still finite).
		idx.set_bm25_params(f32::NAN, f32::NAN);
		assert!(idx.search("alpha", 10).iter().all(|h| h.score.is_finite()), "NaN ignored, not applied");
	}

	#[test]
	fn search_ranks_by_bm25_and_excludes_nonmatching_docs() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.insert("d1", "the quick brown fox");
		idx.insert("d2", "lazy dog programming");
		idx.insert("d3", "quick quick fox");

		let hits = idx.search("quick fox", 10);
		assert_eq!(hits.len(), 2, "only docs containing a query term score");
		assert_eq!(hits[0].entity_id, "d3", "higher term frequency ranks first");
		assert_eq!(hits[1].entity_id, "d1");
		assert!(!hits.iter().any(|h| h.entity_id == "d2"), "d2 shares no terms");
	}

	#[test]
	fn search_filtered_drops_nonmatching_before_truncation() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		// All five docs match the query term; only the "keep_" ones pass the keep.
		idx.insert("drop_a", "rust rust rust"); // high tf -> tops an unfiltered search
		idx.insert("drop_b", "rust rust");
		idx.insert("keep_1", "rust");
		idx.insert("keep_2", "rust ownership");
		idx.insert("drop_c", "rust borrow");

		// Unfiltered top-1 is a high-tf drop doc.
		let top1 = idx.search("rust", 1);
		assert!(top1[0].entity_id.starts_with("drop_"), "unfiltered top-1: {}", top1[0].entity_id);

		// Filtered top-1: higher-scoring non-matching docs are removed BEFORE the
		// truncate, so a matching doc is returned — not an empty/fewer-than-1 result.
		let keep = |id: &str| id.starts_with("keep_");
		let f = idx.search_filtered("rust", 1, &keep);
		assert_eq!(f.len(), 1, "still a full k=1 after filtering");
		assert!(f[0].entity_id.starts_with("keep_"), "only matching docs survive: {}", f[0].entity_id);

		// k beyond the match count returns exactly the matches.
		let want: std::collections::HashSet<String> =
			["keep_1", "keep_2"].iter().map(|s| s.to_string()).collect();
		let got: std::collections::HashSet<String> =
			idx.search_filtered("rust", 10, &keep).into_iter().map(|h| h.entity_id).collect();
		assert_eq!(got, want, "filtered to all matches");

		// search delegates to search_filtered with an always-true keep: unchanged.
		assert_eq!(idx.search("rust", 10).len(), 5, "unfiltered returns all 5 docs");
	}

	#[test]
	fn search_empty_query_or_zero_k_is_empty() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.insert("d1", "hello world");
		assert!(idx.search("", 10).is_empty(), "empty query -> no hits");
		assert!(idx.search("hello", 0).is_empty(), "k=0 -> no hits");
		assert!(idx.search("absent", 10).is_empty(), "unindexed term -> no hits");
	}

	#[test]
	fn insert_is_an_idempotent_upsert() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.insert("d1", "alpha beta");
		idx.insert("d1", "alpha beta"); // re-insert same id must not double-count
		assert_eq!(idx.doc_count(), 1, "re-inserting an id keeps one document");
		idx.insert("d1", "gamma"); // upsert to new text -> old terms gone
		assert!(idx.search("alpha", 10).is_empty(), "stale terms removed on upsert");
		assert_eq!(idx.search("gamma", 10).len(), 1);
	}

	#[test]
	fn remove_drops_the_document() {
		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.insert("d1", "alpha");
		idx.insert("d2", "alpha");
		idx.remove("d1");
		assert_eq!(idx.doc_count(), 1);
		let hits = idx.search("alpha", 10);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].entity_id, "d2");
	}

	#[test]
	fn rebuild_from_graph_indexes_every_nonempty_entity() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert(
			"e1".into(),
			Entity { id: "e1".into(), statements: vec!["quick brown fox".into()], ..Default::default() },
		);
		k.entities.insert(
			"e2".into(),
			Entity { id: "e2".into(), statements: vec!["lazy dog".into()], ..Default::default() },
		);
		// Empty-statement entity must be skipped, not indexed as a zero-len doc.
		k.entities.insert("e3".into(), Entity { id: "e3".into(), ..Default::default() });
		g.kerns.insert("k".into(), k);

		let idx = LexicalIndex::new_in_ram(1.2, 0.75);
		idx.rebuild_from_graph(&g);

		assert_eq!(idx.doc_count(), 2, "only the two non-empty entities are indexed");
		let hits = idx.search("fox", 10);
		assert_eq!(hits.len(), 1);
		assert_eq!(hits[0].entity_id, "e1");
	}
}
