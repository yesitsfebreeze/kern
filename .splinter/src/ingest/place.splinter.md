# src/ingest/place.rs — commentary

- `place_chunks` / question seeding: relocated to `tick::tasks::do_seed_questions`, deferred via `DeferQuestionsFn` — one implementation, one owner (the tick). It was previously a blocking reason-LLM call per chunk on the commit path, which made the worker LLM-bound and starved every queued ingest (measured: a one-line sync ingest waited 69.7 minutes).

Second-pass migration (from source comments):

- `place_chunks` tests: an empty chunk vector means the embed failed for that chunk and must be skipped, not placed (`place_chunks_skips_empty_vectors`). The defer hook fires exactly once per placed, non-deduped chunk (`place_chunks_defers_question_seeding_via_the_hook`).
