# src/mcp/tools_admin.rs — commentary

- `AnchorArgs`/`DescArgs`/`PulseArgs`: argument DTOs are hoisted to module level (out of the method bodies) so they can be reused and unit-tested in isolation.
- `tool_schemas`: schemas are co-located with their handlers so schema and handler can't silently drift.