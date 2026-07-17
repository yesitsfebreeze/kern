# src/commands/reembed.rs — commentary

Second-pass migration (rationale moved out of the source comments):

- Module purpose: the embedding dimension locks into the graph at first ingest, so switching models (e.g. nomic-embed-text 768-d → qwen3-embedding) requires re-embedding every stored vector. Run after changing `[embed] model` in kern.toml, with the daemon stopped — the command writes the graph directly.
- Client construction: reason/answer endpoints are `Endpoint::default()` on purpose; this command is embed-only.
- Entity collection is graph-wide in a stable order (ids/texts kept parallel) before any embed call.
- `reembed_cold`: atomic by construction — vectors are reassigned in memory and committed only if EVERY batch succeeds, so a failure leaves the cold tier fully unchanged, never a partial-dimension mix that would corrupt cold search. On failure the error names the offending batch and exactly how many cold entities were left un-re-embedded, so the caller reports a precise partial-success state instead of a generic abort. `Ok(n)` = cold entities re-embedded, `0` when there is no store or nothing is cold.
- Failure print on the cold path: the hot graph is already on the new model while cold failed and was left intact; the message states exactly what is stale so the operator can re-run, rather than printing a misleading "complete".
- Tests: both stubs return exactly ONE embedding regardless of input count, so a 2-input batch trips the `vs.len() != chunk.len()` count guard in `embed_all` / bails `reembed_cold`. The cold test seeds two entities with a recognizable OLD vector and asserts (a) precise partial-success reporting ("2 of 2" + "cold tier untouched") and (b) atomicity — the tier still holds the ORIGINAL vectors, never the stub's.
`kern reembed` re-embeds every stored vector after an [embed] model change (embedding dimension locks at first ingest). Daemon must be stopped — it writes the graph directly. reembed_cold re-embeds cold-tier entities too because old-dim vectors silently drop from cold search; it is atomic, committing only if EVERY batch succeeds.
