# src/trnsprt/src/kern_rpc/svc.rs — commentary

Sub-agents and other clients hold a `KernRpcClient`; kern (or a mock for tests) implements `KernRpc`. The service is a sibling to `SearchSvc` and intentionally shares several DTOs with it (`EntityRef`, `EntityKindLite`, `EdgeKind`, `NeighborsReq/Res`). Session/fork orchestration is intentionally NOT part of `KernRpc` — kern stays unaware of any client's session model. The macro-generated client + server plumbing is exercised end-to-end by `tests/kern_rpc.rs`.

- `call_tool`: exists for the `kern mcp` proxy subprocess to relay stdio MCP `tools/call` requests over kern.sock without enumerating each tool as a typed method.
- `list_tools`: forwarded by the `kern mcp` proxy so a pane's `tools/list` reflects what the daemon actually exposes (e.g. the mux comms tools), not the proxy's static catalogue.

Second-pass migration: module `//!` doc compressed to the macro-output inventory; `call_tool` method doc trimmed (proxy rationale is recorded above).
Design notes (moved from source comments during comment sweep):
- KernRpc service definition — the service! macro generates the trait, KernRpcClient<C>, and the serve_kern_rpc(channel, handler) loop.
- RPC contracts: query = retrieval pipeline, ranked hits + optional LLM answer. ingest = ingest text/URI as an Entity, returns the new entity id (or a doc id if the call ran async). link = create a typed Reason edge between two entities. neighbors = depth-1 (clamped to 3) typed graph walk, reuses SearchSvc's NeighborsReq/Res. forget = hard-delete an entity by id (prefix-matched). degrade = decay confidence on an entity by id (prefix-matched). health = daemon liveness + summary counters. anchor = manage anchors (named top-level buckets): list/add/remove. descriptor = add or remove a descriptor classifier. pulse = fire a stigmergic pulse through the root kern. call_tool = generic MCP tool dispatch via the daemon's mcp::Server::call_tool, returns the full { content, isError? } envelope. list_tools = enumerate the daemon's live MCP tool schemas.
