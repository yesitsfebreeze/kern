# src/commands/ingest_cmd.rs — commentary

- `cmd_ingest` (no defer hooks): questions are enrichment, not data — the daemon's tick will NOT backfill question seeding for CLI-ingested entities. That gap is the documented trade for keeping the ingest worker free of reason-LLM calls.
- `cmd_ingest` (optimistic retry): a live daemon (or parallel CLI) is the second writer the "never two writers on one data_dir" hazard warns about. Blind overwriting is exactly how the daemon used to wipe committed ingests; divergence is detected via the store epoch instead. The reload path re-embeds, but only on an actual race, which is rare.
cmd_ingest owns persistence itself via the guarded flush (Worker built with no save_fn); no tick loop runs here, so question seeding is skipped. WRITE_RETRIES (5) caps a pathological optimistic-concurrency spin: each round reloads committed state and re-applies after a lost flush race.
