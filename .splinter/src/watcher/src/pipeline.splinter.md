# src/watcher/src/pipeline.rs — commentary

- `MAX_INGEST_BYTES`: 1 MB cap rationale — the search index is for source-shaped text, not blobs.
- `IngestSink`: implemented by the kern wiring (build slice F); this crate stays kern-free.
- `IngestPipeline`: `Deleted` is ignored here because build slice E scoped this crate to *content* ingest only; kern owns deletion via a separate call path.
Second-pass migration:
- `IngestPipeline` event mapping (moved from struct doc): `Created`/`Modified` → read file (≤ MAX_INGEST_BYTES) → ingest; `Renamed { from, to }` → treated as `Created(to)`; `Deleted` → no read (see above). Oversize and non-UTF-8 files skip silently, logged at `debug`.
- `handle` is exposed (next to `run`) for tests and synchronous callers.
- `file_uri`: Windows canonical paths come back as `\\?\C:\foo`; the normalisation strips the `//?/` prefix so the drive path becomes `file:///C:/..`.
- Test `renamed_with_non_utf8_from_reads_the_to_path`: build_record uses the `to` endpoint of a rename; the `from` path is never read or stringified, so a non-UTF-8 `from` must not affect the record.
