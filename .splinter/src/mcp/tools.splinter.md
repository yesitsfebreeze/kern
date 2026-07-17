# src/mcp/tools.rs — commentary

- `tool_definitions`: the catalogue is assembled from each handler module so a tool's schema lives next to the `tool_*` impl that serves it — schema and handler can't silently drift across files.
tool_definitions() order is intentional and asserted by the test definitions_are_well_formed_and_complete (expected order: query, ingest, link, forget, degrade, health, anchor, descriptor, pulse, gc).
