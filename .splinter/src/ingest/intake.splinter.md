The daemon is the single graph owner, so intake ingest happens in-process with no CLI race. MCP `ingest` durable ack status is `"accepted"`.

- `prune_done`: the graph is the durable copy after ingest — the `done/` archive is only a transient audit trail; sweeping it each drain cycle bounds disk/inode growth on a daemon that runs indefinitely.
- `drain_once`: split out of `run`'s poll loop so the full intake→distill→ingest→archive path is unit-testable without spawning the never-returning loop. Also hosts the durable direct-ingest lane: MCP `ingest` payloads persisted under `<intake>/direct/` replay through the same worker (verbatim — no distill) and archive into `direct/done/` on success.

Second-pass migration (from source comments):

- `extract_claims` outage guard: an LLM outage (empty output) returns `None` so the delta stays in the intake for retry and is never archived. This was a real data-loss bug — the outage path archived the delta unread. `extract_returns_none_on_llm_outage` is the regression guard; `extract_returns_some_on_genuine_no_claims` pins the other side (`"[]"` = nothing worth keeping = `Some([])`, archivable).
