# src/retrieval/score.rs — commentary

- `commit_access_ids_with_half_life`: history — the query path used to stamp accesses inline under a brief write lock; deferred to the `CommitAccess` tick task so the interactive query path never takes a write lock. Without the live write-back, self-compaction is inert on a standalone node (the GC staleness clock never advances).
- `matches_filter`: the "no timestamp is not excluded by since/before" rule deliberately preserved the pre-refactor `is_none_or` semantics.
Second-pass migration:

- `commit_access_ids_with_half_life` epoch invariant (resolves the `(see note)` on its doc comment): the write goes `g.kerns.get_mut(kern_id)` -> `k.entities.get_mut(id)`, deliberately NOT through `GraphGnn::get_mut`, because `get_mut` bumps the global mutation epoch. That epoch is what invalidates `retrieval::cache::QueryCache` (any bump flushes every entry). An access stamp is a *read* side effect — bumping the epoch for it would make every served query invalidate the cache that just served it, so the cache could never hit and the ~30 s LLM path would be paid on every repeat query. The bypass is sound because access stamps (CRDT counter, `accessed_at`, heat deposit) do not change any content a cached result rendered.
- `filter_delivery` MMR interaction (backs the inline `filter_delivery_keeps_mmr_pool_when_mmr_enabled` pointer): with MMR enabled the larger `mmr_pool_size` is kept rather than truncating to `max_deliver_results` here. Truncating to the delivery cap at this stage would leave MMR nothing to choose among — its len-guard would no-op and diversification would silently become a pass-through, which is exactly the regression the named test pins.
QueryOptions field semantics:
- source: legacy free-form source-system filter, compared against Source::system().
- kind: typed entity-kind filter; None disables.
- scheme: URI scheme filter ("file"/"ticket"/etc); None disables.
- as_of: bi-temporal WORLD-TIME point query — keep entities whose [valid_from, valid_to) covers this instant. DISTINCT from valid_at, which gates TTL expiry. Two similar SystemTime fields with different meaning — don't conflate.
- include_history: include Superseded entities via Supersedes chain walks from active hits — a tool-layer walk, NOT a per-entity filter (which is why it is absent from is_active/matches_filter; the ANN never holds superseded entities).
- is_active(): true iff any metadata filter is set. sort/ascending are presentation, not filters. False lets callers take the cheaper unfiltered ANN path.

matches_filter behavior details:
- since/before gate on created_at; an entity with no timestamp is not excluded by either bound.
- valid_at: an entity whose validity has expired before the query instant is filtered out; no expiry means always valid.
- matches_filter is THE single filter predicate shared by post-filtering (apply_query_options) and pre-filtered ANN search (search_all_filtered) — the two must never diverge.

filter_delivery: with MMR on, keep the larger MMR pool (mmr_pool_size.max(max_deliver_results)); truncating straight to the delivery cap here would make MMR's len-guard a no-op.

Access stamping:
- stamp_access is the single access-stamp: bump the CRDT access counter, set accessed_at, deposit heat. Shared by commit_access (result copies) and commit_access_ids (live graph).
- commit_access_ids goes through g.kerns directly, NOT get_mut, because an access stamp must not bump the mutation epoch (it would invalidate the query cache).
