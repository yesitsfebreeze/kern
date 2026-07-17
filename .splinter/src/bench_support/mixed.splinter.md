# src/bench_support/mixed.rs — commentary

This bench is the A/B lever for the lock-contention work (parking_lot swap, read-only queries, snapshot-then-flush persist): run it before and after a change and compare the tail (p99/max read stall).

Second-pass migration:
- `GraphLock` (doc deleted): the alias exists so the bench follows whatever lock library the daemon uses — flipping the one `type GraphLock = ...` line re-points every thread without touching the harness body. It currently aliases `parking_lot::RwLock<GraphGnn>`.
- Thread-role labels ("Writers:", "Persist thread:", "Readers:") were deleted from `measure_mixed`'s scope block — the spawned closures are self-evident. Structure for reference: `writers` accept-threads building synthetic entities from the corpus vectors, one persist thread doing a guarded save every ~2s, and `readers` threads on the real locked query path that each time every query and return their samples for the harness to pool into percentiles.
- Kept inline: the tempfile-is-a-dev-dependency workaround (the bin cannot use it, so the temp dir is minted by hand) and the precompute-embeddings-once invariant — a thread re-embedding inside the loop would measure the embedder, not the lock.
