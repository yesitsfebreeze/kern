//! Rephrase-candidate synthesis: find pairs of *near*-duplicate thoughts —
//! similar enough to be worth an LLM rephrase/merge, but below the exact-dup
//! threshold handled by ingest-time dedup. The synthesis tick consumes the
//! returned triples and proposes merges.

use crate::base::graph::GraphGnn;
use crate::ingest::Config;

pub type RephraseCandidate = (String, String, f32);

/// Above this many total entities the all-pairs ANN sweep (`O(n·k)` searches) is
/// too expensive to run inline on the ingest thread, so synthesis is skipped for
/// the pass and retried later. A safety bound, not a tuning knob.
const MAX_SYNTHESIS_ENTITIES: usize = 50_000;

/// Scan every embedded thought, ANN-search its neighbours, and return canonical
/// `(a, b, similarity)` pairs whose similarity falls in the **open** window
/// (`cfg.rephrase_lower`, `cfg.rephrase_upper`). Ids are ordered lexicographically
/// and de-duplicated so each unordered pair is reported at most once. The caller
/// (the synthesis tick) uses each triple to propose an LLM rephrase/merge of the
/// two thoughts.
///
/// Returns empty if the graph exceeds [`MAX_SYNTHESIS_ENTITIES`] — too large to
/// sweep inline without blocking the ingest thread.
pub fn find_rephrase_candidates(graph: &GraphGnn, cfg: &Config) -> Vec<RephraseCandidate> {
	let total: usize = graph.map().values().map(|k| k.entities.len()).sum();
	if total > MAX_SYNTHESIS_ENTITIES {
		tracing::warn!(
			target: "kern.synthesis",
			entities = total,
			cap = MAX_SYNTHESIS_ENTITIES,
			"skipping rephrase synthesis: graph too large for an inline all-pairs ANN sweep"
		);
		return Vec::new();
	}

	let mut seen = std::collections::HashSet::<(String, String)>::new();
	let mut out = Vec::new();

	for kern in graph.map().values() {
		for t in kern.entities.values() {
			if t.vector.is_empty() {
				continue;
			}
			let hits = graph.entity_idx.search(&t.vector, cfg.hnsw_k, cfg.hnsw_ef);
			for h in hits {
				consider_pair(&t.id, &h.id, h.score, cfg, &mut seen, &mut out);
			}
		}
	}

	out
}

/// Decide whether `(t_id, hit_id)` is a fresh, in-window rephrase pair and, if so,
/// record it. Skips self-matches and any similarity outside the open window;
/// canonicalizes id order (lexicographically smaller first) so `a–b` and `b–a`
/// collapse to one entry; de-duplicates via `seen`. Pure given its arguments —
/// no graph or index access — so the windowing/canonicalization/dedup logic is
/// unit-testable apart from the ANN sweep.
fn consider_pair(
	t_id: &str,
	hit_id: &str,
	sim: f64,
	cfg: &Config,
	seen: &mut std::collections::HashSet<(String, String)>,
	out: &mut Vec<RephraseCandidate>,
) {
	if hit_id == t_id {
		return;
	}
	if sim <= cfg.rephrase_lower || sim >= cfg.rephrase_upper {
		return;
	}
	let (a, b) = if t_id < hit_id {
		(t_id.to_string(), hit_id.to_string())
	} else {
		(hit_id.to_string(), t_id.to_string())
	};
	if seen.insert((a.clone(), b.clone())) {
		out.push((a, b, sim as f32));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashSet;

	fn cfg() -> Config {
		Config {
			rephrase_lower: 0.5,
			rephrase_upper: 0.95,
			..Default::default()
		}
	}

	#[test]
	fn in_window_pair_is_canonicalized_and_recorded() {
		let (mut seen, mut out) = (HashSet::new(), Vec::new());
		// Pass the ids in non-canonical order; output must be (a, b) sorted.
		consider_pair("z", "a", 0.7, &cfg(), &mut seen, &mut out);
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].0, "a");
		assert_eq!(out[0].1, "z");
		assert!((out[0].2 - 0.7).abs() < 1e-6);
	}

	#[test]
	fn similarity_outside_open_window_is_skipped() {
		let (mut seen, mut out) = (HashSet::new(), Vec::new());
		consider_pair("a", "b", 0.5, &cfg(), &mut seen, &mut out); // == lower bound -> excluded
		consider_pair("a", "b", 0.95, &cfg(), &mut seen, &mut out); // == upper bound -> excluded
		consider_pair("a", "b", 0.2, &cfg(), &mut seen, &mut out); // below window
		consider_pair("a", "b", 0.99, &cfg(), &mut seen, &mut out); // above window
		assert!(
			out.is_empty(),
			"boundary and out-of-window sims are excluded"
		);
	}

	#[test]
	fn self_match_is_skipped() {
		let (mut seen, mut out) = (HashSet::new(), Vec::new());
		consider_pair("a", "a", 0.7, &cfg(), &mut seen, &mut out);
		assert!(out.is_empty());
	}

	#[test]
	fn seen_set_makes_pair_idempotent_regardless_of_order() {
		let (mut seen, mut out) = (HashSet::new(), Vec::new());
		consider_pair("a", "b", 0.7, &cfg(), &mut seen, &mut out);
		consider_pair("a", "b", 0.8, &cfg(), &mut seen, &mut out); // same pair again
		consider_pair("b", "a", 0.6, &cfg(), &mut seen, &mut out); // reversed order
		assert_eq!(out.len(), 1, "an unordered pair is recorded exactly once");
		// First-write wins for the similarity value.
		assert!((out[0].2 - 0.7).abs() < 1e-6);
	}
}
