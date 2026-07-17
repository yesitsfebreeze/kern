# src/trnsprt/src/server.rs — commentary

Second-pass migration:

- `rpc_code_message` doc 3 lines -> 1, keeping the mapping rule (`Rpc` carries its own code; everything else collapses to a generic -32000 server error). It is shared by the `tools/call` and `handle_method` error paths.
- Kept inline: `McpServer::handle_method`'s "return `None` to fall through to method-not-found" (a contract the signature can't express), `extra_capabilities`'s shape example (it returns a raw `Value`), the `run` test-helper's one-liner, and the `// Must NOT be processed` oracle in `serve_rw_runs_initialize_list_call_then_stops_at_shutdown` — that frame (id 5) guards the regression where shutdown fails to return before reading the next line.
- Test `Mock` is local rather than `test_utils::AdderServer` because of trnsprt's dev-dep cycle — see the `http.rs` note.
Design notes (moved from source comments during comment sweep):
- McpServer::extra_capabilities advertises MCP capabilities beyond `tools` (e.g. {"resources": {}, "prompts": {}}); merged into the initialize reply's capabilities.
- McpServer::handle_method handles MCP methods not covered by the standard tool/init/shutdown dispatch; returning None falls through to method-not-found (-32601).
- rpc_code_message: McpError::Rpc carries its own JSON-RPC code; every other error arm collapses to -32000.
