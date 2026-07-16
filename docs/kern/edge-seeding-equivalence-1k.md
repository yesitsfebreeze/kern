# Bench edge-seeding: pairwise vs ANN top-k — 1k equivalence + timing

Decision record for closing the "ANN top-k edge seeding" task **no-change**. The
bench keeps the exhaustive O(n^2) pairwise similarity seeding in
`src/bench_support/build.rs`; the ANN top-k alternative was implemented, measured,
and **rejected as a net build-time regression**.

(Lives here, not under `traces/`, because `traces/` is gitignored — bench traces
are regenerated deterministically rather than committed.)

## Edge-set equivalence (1000-doc synthetic trace)

Trace: 40 clusters of 3 near-identical docs (8 shared tokens + 1 unique each,
cosine ~0.89 > the 0.5 floor) among 880 unrelated singles (disjoint vocab,
cosine ~0). Only intra-cluster pairs clear the floor.

| edge set | edges | fingerprint (`content_hash` of sorted reason ids) |
|---|---|---|
| old — pairwise O(n^2) brute force (what the bench uses) | 120 | `adeea312010493d6e9c50caf95b1aa756cd8abef4985144d1d7e546ce68e7847` |
| new — ANN top-k, K=64 ef=256, exact-cosine recompute    | 120 | `adeea312010493d6e9c50caf95b1aa756cd8abef4985144d1d7e546ce68e7847` |

Diff: **identical** (0 added, 0 removed). ANN recovers exactly the pairwise set
because a similarity edge only ever connects near-duplicate docs (cosine ~1),
which always fall inside the top-k probe, and the exact cosine is recomputed per
candidate so the floor decision and stored score are bit-identical.

Re-verify (test lives in the bench build module, part of the perf-campaign WIP):
`cargo test --release --features bench --lib bench_support::build::tests::pairwise_seeding_matches_ann_top_k_1k`

## Timing — why ANN was rejected (10k docs, build-only via `retrieval_bench --memory`)

| seeding | build wall |
|---|---|
| pairwise (kept) | ~15-17s |
| ANN top-k | ~22.5s (regression) |

ANN is slower because it needs `rebuild_index()` **twice** (once so the entity
index exists to probe, once so the seeded reasons enter the reason index). HNSW
construction — not the O(n^2) scan — dominates the build (it runs at ~120% CPU,
i.e. sequential-bound; the parallel O(n^2) cosine scan is a small slice).

The `<1min` build target is **already met** by the pairwise seeding (~16s). The
earlier "~5min" figure was `retrieval_bench --all`'s query phases (latency +
throughput sweeps at 10k docs), not the graph build.

Real lever for faster 10k builds, if wanted: `graph.rs` `rebuild_index` /
HNSW construction (parallelize, or id-stable insert order) — a separate slice,
out of scope for the bench build module.
