# src/bench_support/compare.rs — commentary

Phase 1b of the Qdrant-baseline SPEC (`docs/superpowers/specs/2026-06-12-qdrant-baseline-harness-design.md`). The whole point of the seam: when a feature-gated `QdrantBackend` is added it slots into `compare` with zero metric-code changes.

- `Corpus::synthetic`: each of `n_docs` documents draws 8 tokens from a 200-term shared vocabulary (vectors overlap, so ANN recall is non-trivial — not a toy orthogonal set); each query takes a 4-token subset of one target doc's tokens, so the target is the intended best match while real overlap from other docs keeps recall discriminating.

Second-pass migration:
- `CompareQuery` / `BackendReport` / `K` (docs deleted): a CompareQuery is the pre-embedded query vector + ground-truth `expected_ids` + optional kind filter; a BackendReport is one backend's scored row in the head-to-head table; `K` = 10 is the `k` for recall@k / NDCG@k in the baseline.
- `Corpus::synthetic`: doc reduced to the determinism contract; the corpus-shape rationale is already recorded above in this note.
- Test thresholds: `synthetic_corpus_drives_a_scale_comparison` uses 500 docs / 50 queries because the target shares the query's tokens, so it should land in the top-10 for a substantial fraction of queries — the `> 0.3` floor is deliberately discriminating rather than trivially perfect. `kern_ann_recall_tracks_exact_brute_force` treats brute force as the exact-NN ceiling and requires kern's HNSW to reach >=80% of it: the "keep the DiskANN recall@k edge" check.
- `identical_backends_produce_identical_quality_and_memory` is the apples-to-apples guarantee that the harness adds no per-backend bias; latency is wall-clock and excluded from it (that exclusion stays inline).
