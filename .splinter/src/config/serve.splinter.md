# splinter: src/config/serve.rs

Second-pass migration:
- `ServeConfig` fields: `addr` is the primary client-facing endpoint (HTTP RPC + MCP-over-HTTP); `core_addr` is the kern_rpc surface used by sub-agents and other in-host processes; `mcp_sse` is the push transport for MCP clients; `gossip` carries discovery + state sync between peer daemons.
- `validate`: exists to catch an obviously-broken layout BEFORE bind — two TCP listeners on one port otherwise fail at startup with a confusing OS-level error. It is a structural check only, not a tuning oracle. Unparseable `host:port` is deliberately skipped so the real bind surfaces the error; port 0 is skipped because ephemeral ports never truly clash (two `:0` entries must not collide). Returns the offending port plus both field names.
