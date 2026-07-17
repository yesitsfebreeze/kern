# src/trnsprt/src/kern_rpc/svc.rs — commentary

Sub-agents and other clients hold a `KernRpcClient`; kern (or a mock for tests) implements `KernRpc`. The service is a sibling to `SearchSvc` and intentionally shares several DTOs with it (`EntityRef`, `EntityKindLite`, `EdgeKind`, `NeighborsReq/Res`). Session/fork orchestration is intentionally NOT part of `KernRpc` — kern stays unaware of any client's session model. The macro-generated client + server plumbing is exercised end-to-end by `tests/kern_rpc.rs`.

- `call_tool`: exists for the `kern mcp` proxy subprocess to relay stdio MCP `tools/call` requests over kern.sock without enumerating each tool as a typed method.
- `list_tools`: forwarded by the `kern mcp` proxy so a pane's `tools/list` reflects what the daemon actually exposes (e.g. the mux comms tools), not the proxy's static catalogue.

Second-pass migration: module `//!` doc compressed to the macro-output inventory; `call_tool` method doc trimmed (proxy rationale is recorded above).
