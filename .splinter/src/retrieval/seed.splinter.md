# src/retrieval/seed.rs — commentary

- `seed_with_important`: exists because Hybrid retrieval needs the importance scan a second time as its own RRF list in `answer::fuse_hybrid_seeds`; the caller scans once and threads the result into both consumers. `seed` is the scan-then-delegate wrapper for callers that don't reuse the list. Requires the scan to be deterministic across calls (pinned by `seed_important_is_deterministic_at_scale`).
- `seed_with_important` filtered-ANN path: fixed the post-filtering coverage bug (unfiltered top-k post-filtered down to fewer-than-k matching hits). With no active filter the unfiltered path is taken unchanged, so unfiltered queries are byte-identical to the pre-fix behavior. The `keep` predicate resolves candidate ids to entities by reference (no clone).
- `seed_important`: the filter check is cheap field comparisons and runs before the cosine, so non-matching entities also skip the dot product.
Second-pass migration:

- `seed` scope boundary (resolves the `(see note)` on its doc comment): `seed`/`seed_with_important` produce only the dense half — vector ANN (the reason-vector ANN in `Mode::Reason`) merged with the importance list. The lexical (BM25) and PageRank lists are NOT blended here; they are fused on top by `answer::fuse_hybrid_seeds` via weighted RRF, which is also where the query-independent priors get down-weighted by `cfg.rrf_global_weight`. Keeping the blend out of `seed` is what lets the same seed function serve both the hybrid path and callers that want the dense seed alone.
- `active_kind_filter_seeds_matches_post_filtering_would_miss` setup (moved out of the test body): the fixture is 30 Claims at cosine 1.0 against the query and 3 Facts at ~0.994, so the Claims bury the Facts below any unfiltered top-k — that is what makes post-filtering visibly lose matching hits and pre-filtering keep them. `important_min_cosine: 1.5` is the isolation knob: no cosine can reach 1.5, so the importance list comes back empty and the test observes the dense-seed path alone rather than a dense+importance merge.

# Ratings — scope: src/retrieval/seed.rs

Scope rating: 8/10 — three seed strategies (important/lexical/reason) feeding the hybrid fusion. Parallel importance scan with filter-aware gating. Two sort-by-score paths lacked id tiebreaks (non-deterministic on cosine ties); fixed to use cmp_rank.

## Function ratings

- `seed_important` — 7/10→9/10: O(N) parallel scan with cosine + access gates, filter-aware. Sort was `partial_cmp.unwrap_or(Equal)` (no id tiebreak → non-deterministic on ties); fixed to `cmp_rank` (score desc, id asc), consistent with rrf/merge_hits/lexical/cold_search/union_rank.
- `seed_lexical` — 9/10: delegates to LexicalIndex search (filtered or unfiltered), maps to EntityHit. Filter-before-truncation is correct.
- `seed_by_reason` — 7/10→9/10: reason-edge scan, max-join by `from` entity. Same sort fix as seed_important.
- `seed_with_important` — 8/10: merges important + HNSW search + reason seeds. Correct priority merge.
- `fuse_hybrid_seeds` — 8/10: RRF fuses dense + lexical + important seeds. Reuses `important` scan (run once, threaded in).
- `merge_seeds` — 8/10: priority-ordered seed merge with dedup.
- `matches_keep` — 9/10: filter predicate bridge for lexical search.
- `seed_important_is_deterministic_at_scale` — 9/10: covers determinism at scale; now backed by cmp_rank tiebreak.
