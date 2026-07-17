# src/bench_support/backend.rs — commentary

Phase 1 of the Qdrant-baseline SPEC (`docs/superpowers/specs/2026-06-12-qdrant-baseline-harness-design.md`): this module is the abstraction + kern's reference implementation; the Qdrant adapter and the multi-backend `compare` harness are later phases (compare landed as Phase 1b).

The embedder — not the index — was the confound that dominated earlier recall comparisons; that finding is why the harness insists embeddings are computed once by the caller and shared across backends.

Second-pass migration:
- `KernBackend` (doc deleted): kern's own vector index — HNSW over `entity_idx` — and the reference backend the Qdrant column is measured against.
- `BruteForceBackend` (doc compressed to 2 lines): comparing kern's approximate HNSW against this exact full-scan measures how much recall the ANN gives up versus exact nearest-neighbour — the "keep the DiskANN recall@k edge" check. The O(n)-per-query "baseline, not a contender" caveat stays inline.
- The `query` filtered-during-search (not post-filtered) trap and the "score desc, id asc" deterministic-ranking line stay inline; both guard real regressions.
# src/bench_support/backend.rs — commentary (migrated from source doc comments)

- Module: pluggable vector backend for the Qdrant head-to-head baseline. Embeddings are computed ONCE by the caller, so any recall gap between backends is the index — not the embedder.
- `Doc`: a pre-embedded corpus document; `vector` is kern-native f32. `QueryHit`: a ranked result (entity id + similarity score, descending).
- `VectorBackend` trait: a vector index that can be A/B'd against kern in the baseline harness. `query`'s `kind_filter` must filter DURING the search, not post-filter (post-filtering yields fewer than k — the fewer-than-k fix; tight hazard line kept inline). `vector_bytes` returns vector-payload bytes, a lower bound on RSS, for the memory column.
- `BruteForceBackend`: exact brute-force cosine scan — the ground-truth recall CEILING for kern's ANN. O(n) per query: a baseline, not a contender. It sorts by the same deterministic ranking as the rest of the stack (score desc, id asc, via `cmp_rank`).
- Test `kern_backend_kind_filter_returns_only_matching`: all three docs are equally near; a Fact filter must surface only the Fact (filtered during traversal, so it is not lost behind the closer claims).
