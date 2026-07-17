# src/mcp/tools_admin.rs — commentary

- `AnchorArgs`/`DescArgs`/`PulseArgs`: argument DTOs are hoisted to module level (out of the method bodies) so they can be reused and unit-tested in isolation.
- `tool_schemas`: schemas are co-located with their handlers so schema and handler can't silently drift.
tool_pulse labels the no-op ("no task queue configured") so a caller can distinguish a one-shot CLI Server with no tick queue from a real 0-enqueue pulse. The health_stats aggregation test guards against drift from repl.rs's copy of the same aggregation.
