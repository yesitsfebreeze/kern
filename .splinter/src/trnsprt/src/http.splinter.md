# src/trnsprt/src/http.rs — commentary

Second-pass migration:

- Module doc 4 lines -> 2. `serve_http`'s doc line deleted as a restatement (the `addr` example `"127.0.0.1:3001"` was its only content).
- `handle_post` content-type guard, compressed to 2 lines inline. Full reason: a missing Content-Type header is allowed because many MCP clients omit it; the JSON parse below already rejects a non-JSON body with -32700, so the header check only hardens the *mislabelled-payload* case. It is not the primary defence.
- Test `MockServer` (6 lines -> 2): trnsprt has a dev-dependency CYCLE, `trnsprt -> test-utils -> trnsprt`. Inside trnsprt's own unit-test build, `test_utils::mcp_pipe::AdderServer` implements a *different* `McpServer` trait instance than the harness sees, so a local mock over this crate's own trait is required. The cross-crate `AdderServer` still works in `src/trnsprt/tests/integration.rs`, a separate crate where both sides see the same trnsprt. The same constraint drives the local mocks in `inproc.rs` and `server.rs`.
Design notes (moved from source comments during comment sweep):
- Implements MCP Streamable HTTP transport (2025 spec). POST /mcp = request -> JSON response; GET /mcp = SSE stream for server-initiated notifications (keepalive comments only today).
