# src/mcp/sse.rs — commentary

- Kept as its own tiny module (rather than inlined into `mcp.rs`) so the transport wiring stays separate from the `Server` / tool-dispatch logic.

Recovered from git (killed-agent gap):

- `serve_sse` (wire shape): the 2025 MCP Streamable HTTP transport is POST `/mcp` for request→response and GET `/mcp` for the server-sent-events notification stream. Delegates to `trnsprt::serve_http`.
- `serve_sse` (error contract): returns when the listener errors; the error is propagated to the caller, which logs it (see `commands.rs`).
run_sse is just the kern-side adapter; the actual HTTP server lives in trnsprt. Transport is MCP Streamable HTTP (2025), not a bare WebSocket, despite the module name.
