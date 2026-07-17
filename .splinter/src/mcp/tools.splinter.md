# src/mcp/tools.rs — commentary

- `tool_definitions`: the catalogue is assembled from each handler module so a tool's schema lives next to the `tool_*` impl that serves it — schema and handler can't silently drift across files.