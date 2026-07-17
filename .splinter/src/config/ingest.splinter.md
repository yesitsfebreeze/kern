# splinter: src/config/ingest.rs

Second-pass migration:
- `IngestConfig` mirrors the runtime `ingest::Config` knobs, which additionally carries `ttl_secs` and `valid_from`; both layers default to the shared `INGEST_*` constants in `base::constants` so they cannot drift.
- `dedup_threshold` semantics: cosine-similarity floor — a new vector whose nearest neighbour scores at or above it is merged as a duplicate of that entity instead of inserted. Higher is a stricter "same thought" test.
- `validate` delegates to the runtime `ingest::Config` range check (mapping these serde fields onto it) rather than repeating the `[0,1]` bound, so the two config layers can never validate differently.
