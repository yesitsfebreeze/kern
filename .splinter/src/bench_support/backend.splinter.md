# src/bench_support/backend.rs — commentary

Phase 1 of the Qdrant-baseline SPEC (`docs/superpowers/specs/2026-06-12-qdrant-baseline-harness-design.md`): this module is the abstraction + kern's reference implementation; the Qdrant adapter and the multi-backend `compare` harness are later phases (compare landed as Phase 1b).

The embedder — not the index — was the confound that dominated earlier recall comparisons; that finding is why the harness insists embeddings are computed once by the caller and shared across backends.

Second-pass migration:
- `KernBackend` (doc deleted): kern's own vector index — HNSW over `entity_idx` — and the reference backend the Qdrant column is measured against.
- `BruteForceBackend` (doc compressed to 2 lines): comparing kern's approximate HNSW against this exact full-scan measures how much recall the ANN gives up versus exact nearest-neighbour — the "keep the DiskANN recall@k edge" check. The O(n)-per-query "baseline, not a contender" caveat stays inline.
- The `query` filtered-during-search (not post-filtered) trap and the "score desc, id asc" deterministic-ranking line stay inline; both guard real regressions.
