# src/trnsprt/src/search/mock.rs — commentary

Returns canned hits and previews from a small, hand-curated corpus. Production kern wiring may use the same `fresh` flag to suppress out-of-order frame application in the palette.

- `filter`: trivial by design — production code would invoke kern's fused index. The facet predicate runs before the substring scan so the result set is bounded by the most specific filter first; matters for downstream tests asserting facet semantics on small corpora. Sort is highest-score-first to mirror fused-rank order.
- `neighbors`: edge_kinds filtering is a no-op in the mock unless the caller restricts to kinds excluding `Supports` — then the Claim-class row is dropped purely to demonstrate filtering behaviour (exercised by `neighbors_edge_kind_filter_drops_claim_when_supports_excluded`).
- `preview_dispatches_all_three_variants` (test): guards against a future entity_id arm silently regressing a variant.

Second-pass migration: header `#![allow(clippy::manual_async_fn)]` rationale — explicit `impl Future` mirrors the trait surface; async-fn rewrite adds no value in a test double. In `search`, `fresh = token >= high` uses `>=` (not `==`) so absent tokens (0) still read fresh after `fetch_max` — the stored value is `max(prev, token)`.
Design notes (moved from source comments during comment sweep):
- MockSearchServer is an in-memory SearchSvc handler for tests. cancel_token: only the highest token seen yields fresh:true; older in-flight requests report stale. State is Arc-shared so all clones observe the same cancel-token watermark (high_water is atomic so concurrent calls bump monotonically).
- corpus() is the canned corpus shared by search and neighbors.
- filter(): facets AND across the list; within each Facet the scheme/kind axes also AND when both are set.
- fresh = (token >= high): >= (not >) so absent tokens (0) still count as fresh; == when token==high.
- neighbors demo: restricting edge_kinds without Supports drops the Claim row — a canned demo of edge-kind filtering.
