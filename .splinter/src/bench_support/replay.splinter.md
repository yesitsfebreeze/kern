# splinter: src/bench_support/replay.rs


# src/bench_support/replay.rs — commentary

The trace scoring loop; graph construction lives in `build.rs` (build vs measure).

Second-pass migration:
- `QueryReport::recall10` (doc deleted): recall@10 is order-insensitive coverage of the expected ids in the top-10 — the contract is documented once on `ndcg::recall_at_k`.
- `filtered_query_recovers_a_minority_kind_buried_by_the_majority`: the fixture is 15 Claims + 2 Facts sharing identical text, so id-ascending tie-breaks bury the Facts past top-10 unfiltered (recall 0). A `kind=fact` filter seeds only Facts at source → recall 1.0. This is the proof of the filtered-seed win under the fewer-than-k condition; the fixture-shape comment stays inline, this rationale does not.
- `filtered_query_survives_delivery_pool_truncation`: 60 Claims exceed the ~50 delivery cap, so if the filter ran only AFTER truncation the id-trailing Facts would be cut and recall@10 would collapse. The magic 60 is justified inline.
- `replay_retrieves_relevant_doc_with_positive_ndcg` deliberately asserts recall + positive NDCG rather than exact rank-1 — the full pipeline (graph expansion, MMR, GNN blend) reorders results. That oracle note stays inline; do not tighten it to a rank-1 assertion.
# src/bench_support/replay.rs — commentary (migrated from source doc comments)

- `run_one`: an unparseable `filter_kind` falls back to NO filter (not a wrong-scoring silent match) — `.and_then(EntityKind::parse)` yields None → no `QueryOptions`.
- Test rationale (removed from source but worth keeping):
  - `replay_retrieves_relevant_doc_with_positive_ndcg` asserts recall + positive ranking quality rather than exact rank-1, because the full pipeline (graph expansion, MMR, GNN blend) reorders results.
  - `replay_applies_the_kind_filter_end_to_end`: every doc is a Claim, so a `fact` filter must zero recall while no-filter / `kind=claim` restores it — proving it's the filter, not a broken query.
  - `filtered_query_recovers_a_minority_kind_buried_by_the_majority`: 15 Claims + 2 Facts share identical text; id-ascending ties bury the Facts past top-10 unfiltered, and a `fact` filter surfaces them.
  - `filtered_query_survives_delivery_pool_truncation`: 60 Claims (> the ~50 delivery cap); if the filter ran only AFTER truncation, the id-trailing Facts would be cut and recall@10 would collapse — this guards that the filter runs before truncation.
