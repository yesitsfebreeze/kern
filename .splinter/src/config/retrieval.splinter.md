# src/config/retrieval.rs — commentary

- `mmr_lambda` = 0.75: relevance-dominant (standard MMR region ~0.7). The previous 0.45 over-diversified: on the workload trace it suppressed legitimate multi-relevant cluster hits below rank 10 (recall@10 0.925 -> 1.0, NDCG@10 0.928 -> 0.999 at 0.75; `just bench-workload`, sweep 2026-07-16). Ingest-time dedup (cosine 0.95) already handles true duplicates, so MMR need not.
- `bm25_k1`/`bm25_b`: these fields were dead (unwired) for a while; the range checks in `validate` were added when they were wired into the lexical index, so bad values are caught at config load rather than silently clamped at query time.

Second-pass migration:
- `validate` (`rrf_k` / `seed_k` / `max_deliver_results` group): these are checked because an out-of-range value silently BREAKS retrieval — there is no graceful fallback and no valid use of the bad value. Contrast `important_min_cosine > 1.0`, which is deliberately left unchecked because it is a legitimate "disable" idiom. That distinction is the reason the validate list is selective rather than exhaustive.
Field semantics / derivations removed from source doc comments:
- ModeWeights (content/reason/edge/lexical) must sum to ~1.0; validate flags a deviation > 0.01.
- rrf_global_weight: weighted-RRF multiplier on the query-INDEPENDENT seed lists (importance + PageRank); < 1.0 down-weights global priors, 1.0 is plain RRF.
- hyde_fusion_weight: fused = query*(1-w) + hypo*w, then L2-normalized. Higher trusts the generated hypo more; 0.5 is the symmetric blend.
- query_cache_cap: number of answered queries retained before LRU eviction; 0 disables the cache.
- query_cache_theta: cosine floor for a semantic cache hit; high (~0.97) so only paraphrases share an entry, never merely topical neighbours.

Validation-bound derivations (why each range):
- bm25_b in [0,1]: it is BM25's length-normalisation weight; outside [0,1] the score term `1 - b + b*dl/avgdl` goes negative or over-normalises.
- bm25_k1 >= 0: tf-saturation term; a negative k1 inverts the saturation curve.
- rrf_k >= 0: fuse::rrf scores 1/(rrf_k + rank) with rank >= 1; a negative rrf_k drives the denominator <= 0, inverting or NaN-ing the fusion. rrf_k == 0 is valid RRF and must NOT be flagged.
- pagerank_damping in [0,1).
- seed_k >= 1 (0 seeds nothing), max_deliver_results >= 1 (0 delivers nothing).
validate() is a structural sanity check returning all problems (empty = valid), not a tuning oracle.
