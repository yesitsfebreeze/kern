# src/ingest/mod.rs — commentary

Pipeline map: `Worker` (async mpsc actor, see `worker`) runs each document through 1) `split` — chunk into statement-sized pieces (LLM-assisted, heuristic fallback); 2) `embed` — vectorize document + chunks via the embed endpoint; 3) `place` — insert into the owning kern, consulting `dedup` first so a near-duplicate vector merges into the existing entity instead of spawning a divergent one. `outcome` reports per-document success/partial/failure. Ambient sources feeding the Worker: `intake` (Claude-Code Stop-hook drop-dir) and `file_watcher`; `distill` extracts durable claims from conversation text.

- `Job` re-export: `Job` is the Worker's mpsc message (pub(crate)); re-exported at `ingest::Job` so in-crate producers stay consistent with `ingest::Worker` instead of reaching into `ingest::worker::Job`.
- `stub_one_hot` (test-only embedder): 256-dim one-hot indexed by the content hash's first byte, so distinct seeds land in different slots (cosine ≈ 0) and dodge the dedup check — lets tests place multiple entities without accidental merges.
