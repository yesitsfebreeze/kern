# src/ingest/worker.rs — commentary

- `Worker::new`: the no-reason-LLM design was motivated by a measurement — before deferring LLM-bound steps to the tick, a one-line sync ingest queued 69.7 minutes behind LLM-bound jobs (600s timeout each).
- `run_loop` / `log_outcome`: added after a live incident — an `enqueue`d job that failed vanished without a trace (result_tx None, Outcome + FailureReports dropped unread); observed as 8/8 MCP ingests acked "queued" with nothing in the graph and no log to say why.
- `process`: chunk splitting is heuristic-only because the LLM splitter was one reason-LLM call per document on the commit path; the heuristic fallback was already the common case (any LLM hiccup).
- `finalize_doc_identity`: before carrying the surviving id and Deduped status, dedup merges looked like silent loss — the caller was acked the content hash, which does not exist in the graph after a merge.
- `outcome_log_severity`: kept pure so the status→level mapping is unit-testable; `log_outcome` applies it.

Second-pass migration (from source comments):

- `log_outcome`: the first failure's class+error is inlined into the log line so a single line answers "what happened to doc X" without reaching for a debugger.
- `DeferContradictionFn`: with no hook wired the classification fails open — the `Rephrase` edge simply stands, unclassified.
- `finalize_doc_identity` tests: a merge must return the surviving id, else the caller holds an id that does not exist in the graph (indistinguishable from silent loss). `Partial`/`Failed` are never upgraded to `Deduped`.
