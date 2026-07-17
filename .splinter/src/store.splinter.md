# src/store.rs — commentary

Second-pass migration (rationale moved out of the source comments):

- `Registry.builds` (per-key build locks): only one caller constructs the expensive graph + worker + tick task per data dir; different keys still build in parallel. Without the lock two racers both build and the loser of the final insert is dropped, detaching its already-spawned worker/tick tasks onto an orphaned graph. The `open()` flow is: fast-path read → take the key's build lock → re-check under it (a prior builder may have inserted while we waited) → build + insert.
- `StoreEntry.save_fn`: the ONE persist closure per store graph. `Worker` holds a clone of the same Arc; every other consumer (`mcp::Server`, gossip handlers) must take it from `StoreEntry.save_fn.clone()` rather than build a duplicate closure over the same graph. It routes through `save_graph_guarded`, whose refusal to overwrite a graph another writer grew on disk is the safety net under the single-writer invariant.
- defer hooks (`SeedQuestions`, `ClassifyContradiction`): both exist to keep the ingest commit path embed-bound — no blocking reason/classify LLM call inside the worker. The tick owns all LLM-bound maintenance: it seeds Question edges, and decides UPDATE/CONTRADICTION vs RELATED (kern + rephrase reason id) and supersedes.
- registry race test: the per-key build lock means every racer ends up with the SAME entry and exactly one store is registered — no duplicate build wins.
