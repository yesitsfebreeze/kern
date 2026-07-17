// src/ingest/embed.rs — commentary

Second-pass migration (from source comments):

- `embed_chunks`: one batch call first; on an embed error OR a returned-count mismatch it falls back to per-chunk `embed_with_retry`. A failed chunk's slot holds an empty `Vec` paired with a `FailureReport` — downstream `place::place_chunks` skips empty-vector slots. Empty input short-circuits to `(empty, empty)`.
- `embed_with_retry`: `RETRY_DELAYS_MS = [150, 300, 600]` ms backoff for transient failures; a permanent error bails immediately (no retry storm) and exhausting every retry yields `class: "transient"`. An empty embeddings response (`{"embeddings": []}`) is `EmptyEmbedding` → classified permanent, so it deliberately does NOT trigger the retry storm (`embed_with_retry_treats_an_empty_response_as_permanent`).
