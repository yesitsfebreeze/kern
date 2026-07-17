# src/mcp/tools_mutate.rs — commentary

- `tool_schemas`: schemas are co-located with their handlers so schema and handler can't silently drift.
- `tool_ingest` (Source mapping): the legacy MCP payload maps onto typed `Source` variants — empty `source` (or "inline") collapses to `Inline` (no scheme); the tags "file"/"session"/"agent" route to their matching variants; any other string becomes a `Ticket` system descriptor.
- `tool_ingest` (intake-first history): before the durable intake-first ack, the in-RAM enqueue path acked "queued" while holding the job only in a 64-slot channel; observed live: a daemon restart vaporized 5 acked ingests. That incident motivated the direct-intake-before-ack design.
- `tool_degrade`: the per-edge loop uses one mutable borrow of the kern per edge (refactored from a get-then-get_mut pair).