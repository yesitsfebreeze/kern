# src/ingest/place.rs — commentary

- `place_chunks` / question seeding: relocated to `tick::tasks::do_seed_questions`, deferred via `DeferQuestionsFn` — one implementation, one owner (the tick). It was previously a blocking reason-LLM call per chunk on the commit path, which made the worker LLM-bound and starved every queued ingest (measured: a one-line sync ingest waited 69.7 minutes).

Second-pass migration (from source comments):

- `place_chunks` tests: an empty chunk vector means the embed failed for that chunk and must be skipped, not placed (`place_chunks_skips_empty_vectors`). The defer hook fires exactly once per placed, non-deduped chunk (`place_chunks_defers_question_seeding_via_the_hook`).
- `beta_params_from_confidence`: a clamped `[0,1]` confidence maps to a Beta-Bernoulli prior `Beta(1 + conf, 1 + (1 - conf))` — conf 1.0 → Beta(2,1), 0.0 → Beta(1,2), 0.5 → Beta(1.5,1.5). Out-of-range confidence is clamped before the mapping.
- `new_statement_entity`: the ONLY place the ingest paths materialize an `Entity`, because `Entity` is bincode-positional — two drifting field literals across construction sites would silently corrupt every persisted shard.
- `total_entity_count` test helper: counts entities graph-wide because `accept` routes thoughts into spawned child kerns; a root-only count would miss them.
