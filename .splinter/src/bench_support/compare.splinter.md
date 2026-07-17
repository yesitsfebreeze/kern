# src/bench_support/compare.rs — commentary

Phase 1b of the Qdrant-baseline SPEC (`docs/superpowers/specs/2026-06-12-qdrant-baseline-harness-design.md`). The whole point of the seam: when a feature-gated `QdrantBackend` is added it slots into `compare` with zero metric-code changes.

- `Corpus::synthetic`: each of `n_docs` documents draws 8 tokens from a 200-term shared vocabulary (vectors overlap, so ANN recall is non-trivial — not a toy orthogonal set); each query takes a 4-token subset of one target doc's tokens, so the target is the intended best match while real overlap from other docs keeps recall discriminating.

Second-pass migration:
- `CompareQuery` / `BackendReport` / `K` (docs deleted): a CompareQuery is the pre-embedded query vector + ground-truth `expected_ids` + optional kind filter; a BackendReport is one backend's scored row in the head-to-head table; `K` = 10 is the `k` for recall@k / NDCG@k in the baseline.
- `Corpus::synthetic`: doc reduced to the determinism contract; the corpus-shape rationale is already recorded above in this note.
- Test thresholds: `synthetic_corpus_drives_a_scale_comparison` uses 500 docs / 50 queries because the target shares the query's tokens, so it should land in the top-10 for a substantial fraction of queries — the `> 0.3` floor is deliberately discriminating rather than trivially perfect. `kern_ann_recall_tracks_exact_brute_force` treats brute force as the exact-NN ceiling and requires kern's HNSW to reach >=80% of it: the "keep the DiskANN recall@k edge" check.
- `identical_backends_produce_identical_quality_and_memory` is the apples-to-apples guarantee that the harness adds no per-backend bias; latency is wall-clock and excluded from it (that exclusion stays inline).
# src/bench_support/compare.rs — commentary (migrated from source doc comments)

- Module: multi-backend comparison harness. Indexes one `Corpus` into every `VectorBackend` and scores all through the same `ndcg` + latency code, so any difference between rows is the index/fusion, not the measurement.
- `Corpus`: a shared corpus + query set, embedded once and handed to every backend. `Corpus::synthetic` is deterministic for a given `seed` (xorshift RNG; `s | 1` keeps state non-zero — inline note kept, since xorshift stays stuck at 0 forever from a zero state).
- `compare` scores every backend through identical metric code.
- Test rationale (removed from source): `synthetic_corpus_drives_a_scale_comparison` uses 500 docs — enough to exercise real ANN traversal, not a 4-doc toy. `mean_target_recall_10` = fraction of queries whose expected target lands in the backend's top-10. `disk_spilled_path_recall_tracks_the_in_ram_index` is the I7 regression — the disk tier must not silently lose recall vs the in-RAM HNSW; `disk_backed_graph` sets `disk_threshold = 0` to force the spill so it exercises the real `rebuild_index` routing rather than DiskIndex in isolation. `identical_backends_produce_identical_quality_and_memory` deliberately excludes latency (wall-clock, non-deterministic).
