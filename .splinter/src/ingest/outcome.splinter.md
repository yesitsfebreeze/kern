# src/ingest/outcome.rs — commentary

Second-pass migration (from source comments):

- `OutcomeStatus::Deduped`: kept distinct from `Committed` so a dedup merge is not mistaken for silent loss — the acked content-hash `doc_id` never enters the graph after a merge. In-memory only; never persisted.
- `Outcome.transient_failures`: the transient count is why a `Partial`/`Failed` outcome might still recover on a retry sweep — it separates "retry can fix this" from "retry cannot".
- `FailureReport::document_permanent`: exists so the `scope`/`class` string literals cannot drift between call sites.
