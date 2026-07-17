# src/rpc/kern_rpc_server.rs — commentary

Provenance: the typed KernRpc surface landed as slice J; the kind/scheme/status envelope echo (removing the per-hit graph re-read in `query`) came from Slice Z; the `neighbors` fan-out cap came from card #44.

- `edge_kind_from_reason_int`: lives in the kern adapter rather than `src/trnsprt` because `ReasonKind` is kern-internal and cannot be referenced from the transport crate.
- `query` (edges): enriched relationship edges are extracted from the tool_query envelope so RPC callers see WHY entities are connected, not just THAT they are.
- `str_field`: exists to collapse the `.get().and_then(as_str).unwrap_or().to_string()` chain repeated across the envelope-parsing paths.
- `collect_neighbour_ids`: kept pure so it is unit-testable without standing up an `mcp::Server` graph.
KernRpcHandler wraps the same mcp::Server::tool_* methods as MCP tools/call (identical state transitions), unwrapping the MCP {content:[...]} envelope into the typed wire DTOs of trnsprt::kern_rpc (isError envelope -> Err(message)). Notes carried from stripped comments:
- label_from_snippet: first 80 chars of the snippet, hard cut, no ellipsis (distinct from base::util::truncate).
- edge_kind_tag gives a stable tag for an EdgeKind, used as tool_link's `reason` argument when no explicit text is supplied.
- collect_neighbour_ids: distinct depth-1 neighbour ids from an entity_detail payload — the other endpoint per edge, skipping empties/self-loops, deduped first-seen, capped at max (MAX_NEIGHBORS bounds the N+1 detail loop against data-controlled degree).
- lookup_kind_scheme_status falls back to kern-side defaults when the entity is missing (possible during ingest races) so results still surface.
- serve_kern_rpc_loop: caller owns the singleton-aware bind; per-connection errors are logged, never fatal.
- rpc/mod.rs is the typed-RPC server half of trnsprt::kern_rpc, bound to the per-user kern.sock singleton; import the re-exports from rpc/mod, not kern_rpc_server directly.
