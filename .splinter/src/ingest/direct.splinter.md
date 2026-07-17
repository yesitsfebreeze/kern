# src/ingest/direct.rs — commentary

Origin: the MCP `ingest` tool's fire-and-forget path used to hand a job to the in-RAM worker channel and ack `"queued"` — any daemon exit (watchdog `exit(101)`, crash, operator stop) silently vaporized every queued job after the ack. Observed live: 5 acked ingests lost to one restart. The lane reuses the capture intake's proven durability shape (atomic tmp+rename, intake-before-ack, archive-on-success).

- `intake_direct`: the pid-tagged tmp write is the same shape as the capture hook's offsets write.

Second-pass migration (from source comments):

- `drain_direct_once` poison handling: an unparseable payload is archived rather than left for retry — retrying it forever would wedge the lane behind a payload that can never succeed.
