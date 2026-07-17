# src/bench_support/mixed.rs — commentary

This bench is the A/B lever for the lock-contention work (parking_lot swap, read-only queries, snapshot-then-flush persist): run it before and after a change and compare the tail (p99/max read stall).

Second-pass migration:
- `GraphLock` (doc deleted): the alias exists so the bench follows whatever lock library the daemon uses — flipping the one `type GraphLock = ...` line re-points every thread without touching the harness body. It currently aliases `parking_lot::RwLock<GraphGnn>`.
- Thread-role labels ("Writers:", "Persist thread:", "Readers:") were deleted from `measure_mixed`'s scope block — the spawned closures are self-evident. Structure for reference: `writers` accept-threads building synthetic entities from the corpus vectors, one persist thread doing a guarded save every ~2s, and `readers` threads on the real locked query path that each time every query and return their samples for the harness to pool into percentiles.
- Kept inline: the tempfile-is-a-dev-dependency workaround (the bin cannot use it, so the temp dir is minted by hand) and the precompute-embeddings-once invariant — a thread re-embedding inside the loop would measure the embedder, not the lock.
# src/bench_support/mixed.rs — commentary (migrated from source doc comments)

- Module: mixed read/write/persist contention bench over the REAL locked store paths. The headline number is the worst read stall — what moves when a writer/flush pins the lock.
- `measure_mixed`: spawns `readers` query threads + `writers` accept() threads + one persist thread. The graph is bound to a throwaway on-disk store so persist runs the real LMDB flush. `tempfile` is a dev-dependency (unavailable in this bin), so the temp dir is minted by hand — do not "simplify" to tempfile or the bin build breaks (tight note kept inline). The query set + writer corpus are embedded ONCE up front so no thread re-embeds inside the loop.
- `sleep_interruptible`: sleeps up to `dur` but wakes early (20ms slices) once `stop` is set, so a finished run doesn't wait out the whole 2s persist interval.
- `synthetic_entity`: a synthetic Claim entity in the same shape as `build`'s doc inserts, so `accept` treats it like a real ingested chunk.
