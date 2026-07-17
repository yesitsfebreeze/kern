# src/ingest/dedup.rs — commentary

Second-pass migration (from source comments):

- `update_existing_entity` regression: differing text must NOT mutate the stored `statements`/`vector` under the content-hash id — the id is `content_hash(text)`, so overwriting the text under it makes id and content disagree and breaks CRDT convergence. A `Rephrase` edge records the new phrasing instead (`different_text_preserves_id_invariant_and_records_rephrase`).
- Rephrase idempotency: `reason_id` is content-addressed, so `add_reason` collapses repeat observations of the same phrasing into one edge.
- Kind guard: only a SAME-KIND near-dup is a supersede candidate — a preference must not supersede a fact. Classification is deferred to the tick and fails open when no hook is wired.
