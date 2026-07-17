# src/trnsprt/src/kern_rpc/dto.rs — commentary

Primitive types (`EntityKindLite`, `EntityStatusLite`, `EdgeKind`, `EntityRef`, `NeighborsReq/Res`) are re-imported from the sibling `search` module so the two services share the same wire vocabulary — a Card flowing out of `SearchSvc::search` can be drilled into via `KernRpc::neighbors` without a translation step. All DTOs derive serde Serialize+Deserialize so either codec (line-delimited JSON envelope or length-delimited bincode) can shuttle them; codec choice is per-channel.

- `DegradeReq`: the legacy `strength` field on memory_rpc had no kern-side counterpart and was intentionally dropped.
- `PulseReq`: the legacy `query_id` field on memory_rpc was a Phase-1 holdover the kern side ignored; dropped here.

Second-pass migration — detail moved out of doc comments:

- `SourceLite`: each variant carries the minimum a caller needs to reconstruct the kern-side `Source` enum on the server.
- `QueryRes.hits`: reuses `EntityRef` so a palette frame and a kern_rpc call render the same Card shape.
- `default_true`: needs a named fn because `#[serde(default = "...")]` takes a function *path* — there is no `#[serde(default = true)]` literal form.
- `IngestReq.sync=false` / `IngestRes.entity_id`: the async path queues and returns the content-hash doc id the worker will commit the entity under, before the pipeline has committed it.
- `Anchor`: `entity_id`/`source_uri` identify the addressable anchor (file path, Document id, etc.); `selection` carries the user's literal highlighted text when present (small enough to inline in opening context).
- `CallToolReq`: escape hatch for the `kern mcp` proxy subprocess to relay arbitrary stdio MCP `tools/call` requests to the singleton daemon over `kern.sock` without enumerating every tool as a typed RPC method; the handler forwards `args` verbatim to the daemon's `mcp::Server::call_tool` and returns the full envelope so the proxy pipes it back to stdout unchanged.
- `ListToolsReq/Res`: enumerates the daemon's *live* tool surface so the proxy reflects it rather than serving a static snapshot that omits the mux comms tools (present when the daemon hosts a pane registry).
- `HealthRes`: takes no request payload (the trait method has no arguments).
