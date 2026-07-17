# src/trnsprt/src/registry.rs — commentary

Second-pass migration:

- `LiveServer` and `Registry` docs each trimmed 3 lines -> 2, keeping only the trap in each: the tool snapshot is taken at connect time and goes stale silently (call `refresh_tools` when the tool set changes), and a duplicate `ServerId` errors with `McpError::DuplicateServer` rather than silently replacing the live connection.
- `install` runs the `initialize` handshake and a `list_tools` before the duplicate-id check, so a rejected duplicate has still spawned/handshaken its transport. Not currently a problem (callers drop the result) but worth knowing.
Design notes (moved from source comments during comment sweep):
- LiveServer.tools is a connect-time snapshot that goes stale silently; call refresh_tools() to re-fetch.
- Registry: a duplicate ServerId errors (McpError::DuplicateServer) — it never silently replaces the live connection.
