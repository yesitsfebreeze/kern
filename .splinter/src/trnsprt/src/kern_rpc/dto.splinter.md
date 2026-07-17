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
Design notes (moved from source comments during comment sweep). These DTOs are mirror types that intentionally do NOT depend on the `kern` crate — kern translates at the wire boundary. Wire-compat contracts:

- SourceLite: mirror of kern::Source, one variant per URI scheme. Optional fields collapse to "" on the wire (matches the kern-side Default). SourceLite::scheme() is a stable URI scheme tag matching kern::Source::scheme.
- QueryReq.k: server clamps to a sane maximum. QueryReq.mode: same wire strings as MCP query.mode ("hybrid"|"vector"|"lexical"); empty defaults to "hybrid". QueryReq.answer: if true kern attempts an LLM-synthesised answer alongside hits. QueryReq.kind: optional kind filter, lower-case label (e.g. "fact"). QueryReq.source: optional source-scheme filter (e.g. "file"). QueryReq.cancel_token: cancellation/freshness token, mirrors SearchSvc::SearchReq.
- QueryRes.hits: ranked EntityRef hits (shared with SearchSvc). QueryRes.answer: LLM answer when requested, empty when no LLM configured server-side. QueryRes.fresh: true iff this response was for the most-recent cancel_token the server has seen (mirrors SearchRes::fresh). KEEP in source: default_true() — missing `fresh` on the wire means "not stale"; bool's derived Default (false) would invert that.
- IngestReq.descriptor: descriptor classifier for the ingest pipeline; None skips routing. IngestReq.conf: confidence in [0.0,1.0], server clamps to its agent-source ceiling (Fact tier requires user-source). IngestReq.sync: if true block until commit; else queue and return the content-hash doc id immediately — stable and resolvable on a later read.
- IngestRes.entity_id: new entity id, or (when sync=false) the content-hash doc id returned before the pipeline commits. IngestRes.status: one of "queued"|"ingested"|"duplicate"|"rejected" — matches kern's ingest::outcome::Status::as_str. IngestRes.message: optional pipeline note (rejection reason, dedup pointer).
- LinkReq.reason_kind: mapped server-side to kern's ReasonKind; non-1:1 kinds map to the closest match, with the original kind-name kept in the edge text. LinkReq.text: free-text explanation.
- Anchor: caller-context snapshot carried into a replicated fork. KEEP in source: byte_range is [start,end) over the underlying source bytes.
- ForgetReq.id: matched by prefix server-side (matches existing kern tool_forget semantics). ForgetRes.removed: true iff found and removed.
- DegradeReq: decay confidence on an entity by id (prefix-matched); mirrors kern tool_degrade MCP path. DegradeRes.applied: true iff found and decayed.
- HealthRes: ok = daemon up + store loaded; data_dir = active store data_dir (canonical path string); kerns/entities = totals loaded across all attached stores.
- AnchorReq.action: "list" (default), "add" (needs name+text), or "remove" (needs name). AnchorRes.result: the anchor tool's JSON result, serialized as a string for transport.
- DescriptorReq.action: "add" or "rm" (matches existing kern descriptor CLI).
- PulseReq.strength: pulse strength, 1.0 is the conventional default (see default_pulse_strength). Fires a stigmergic pulse through the root kern.
- CallToolReq: generic MCP tool dispatch for the `kern mcp` proxy; args is the raw tools/call.params.arguments object, forwarded verbatim. CallToolRes.envelope: MCP envelope from daemon-side mcp::Server::call_tool — { "content": [...], "isError": bool }.
- ListToolsRes.tools: the daemon's live tools/list, each entry a raw MCP tool-schema JSON object exactly as mcp::Server::tools_list advertises it.
