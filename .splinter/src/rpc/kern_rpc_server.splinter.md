# src/rpc/kern_rpc_server.rs — commentary

Provenance: the typed KernRpc surface landed as slice J; the kind/scheme/status envelope echo (removing the per-hit graph re-read in `query`) came from Slice Z; the `neighbors` fan-out cap came from card #44.

- `edge_kind_from_reason_int`: lives in the kern adapter rather than `src/trnsprt` because `ReasonKind` is kern-internal and cannot be referenced from the transport crate.
- `query` (edges): enriched relationship edges are extracted from the tool_query envelope so RPC callers see WHY entities are connected, not just THAT they are.
- `str_field`: exists to collapse the `.get().and_then(as_str).unwrap_or().to_string()` chain repeated across the envelope-parsing paths.
- `collect_neighbour_ids`: kept pure so it is unit-testable without standing up an `mcp::Server` graph.