# src/trnsprt/src/kern_rpc/mock.rs — commentary

Mock behavior sketch: tiny in-memory store of `EntityRef`s plus a list of `Reason` edges. `query` does a substring scan over labels; `ingest` appends a fresh row; `link` records an edge; `neighbors` returns every other entity in the corpus filtered by edge kind. Older in-flight `query` requests come back `fresh: false` so palette frames can suppress stale frames.

- `query`: facet filters (kind, scheme) are applied before the substring label match so the result count is bounded by the most specific predicate first; the axes AND together — both must hold when both are set.
- `neighbors`: multi-hop expansion is intentionally out of scope for the test double; the `min(3)` clamp only documents the server's intended ceiling (guarded by `neighbors_returns_only_direct_edges_regardless_of_depth`). Entities are indexed by id into a HashMap so per-edge endpoint lookup is O(1), keeping `neighbors` near-linear in edge count.
- `unrecognised_kind_string_disables_kind_filter` (test): guards against silently eating every hit on a typo'd kind label — the filter must degrade to "no kind constraint", not "match nothing".

Second-pass migration: header `#![allow(clippy::manual_async_fn)]` rationale — the mock returns explicit `impl Future` to mirror the trait surface; the async-fn rewrite adds no value in a test double. `seed()` exists so integration tests get a `query` hit without a prior `ingest`.
