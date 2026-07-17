# src/ingest/file_watcher.rs — commentary

Built as "slice O". The sink deliberately duplicates no placement/dedup logic — it forwards through `Worker::enqueue` so the existing embed → `place_document` pipeline runs unchanged (same shape as slice K's `WorkerSink`). `run` constructs a `FileWatcher` + `IngestPipeline` and pumps events into the sink until the watcher drops (channel close).

- `strip_file_uri`: the non-empty-authority arm (drop the host per RFC 8089) replaced an older behaviour that smuggled `host` into the path.

Second-pass migration (from source comments):

- `strip_file_uri` test: the non-empty-authority case (`file://host/p.rs` -> `/p.rs`) drops the host and keeps the path's leading slash — the RFC 8089 rule is stated on the function itself.
