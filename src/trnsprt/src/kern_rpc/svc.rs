//! `KernRpc` typed-RPC service definition.
//!
//! Driven by the `service!` proc macro from `trnsprt-macros`. Generates
//! - the `KernRpc` trait (one method per RPC),
//! - `KernRpcClient<C>` that owns a [`Channel`](crate::typed::Channel),
//! - `serve_kern_rpc(channel, handler)` server loop.
//!
//! Sub-agents and other clients hold a `KernRpcClient`; kern (or
//! a mock for tests) implements `KernRpc`. The service is a sibling to
//! [`SearchSvc`](crate::search::SearchSvc) and intentionally shares
//! several DTOs with it (`EntityRef`, `EntityKindLite`, `EdgeKind`,
//! `NeighborsReq`/`Res`).
//!
//! Session/fork orchestration is intentionally **not** part of
//! `KernRpc` — kern stays unaware of any client's session model.
//!
//! The macro-generated client + server plumbing is exercised end-to-end by
//! `tests/kern_rpc.rs`.

use super::dto::{
	AnchorReq, AnchorRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
	DescriptorRes, ForgetReq, ForgetRes, HealthRes, IngestReq, IngestRes, LinkReq, LinkRes,
	ListToolsReq, ListToolsRes, NeighborsReq, NeighborsRes, PulseReq, PulseRes, QueryReq, QueryRes,
};

crate::service! {
		/// Typed-RPC surface exposing kern's read+write operations to
		/// sub-agents and other clients.
		pub trait KernRpc {
				/// Retrieval pipeline: ranked hits + optional LLM answer.
				async fn query(req: QueryReq) -> QueryRes;
				/// Ingest text/URI as an Entity. Returns the new entity id
				/// (or a doc id if the call ran async).
				async fn ingest(req: IngestReq) -> IngestRes;
				/// Create a typed Reason edge between two entities.
				async fn link(req: LinkReq) -> LinkRes;
				/// Depth-1 (clamped to 3) typed graph walk. Reuses the same
				/// `NeighborsReq`/`Res` types as `SearchSvc::neighbors`.
				async fn neighbors(req: NeighborsReq) -> NeighborsRes;
				/// Hard-delete an entity by id (prefix-matched).
				async fn forget(req: ForgetReq) -> ForgetRes;
				/// Decay confidence on an entity by id (prefix-matched).
				async fn degrade(req: DegradeReq) -> DegradeRes;
				/// Daemon liveness + summary counters.
				async fn health() -> HealthRes;
				/// Manage anchors (named top-level buckets): list, add, or remove.
				async fn anchor(req: AnchorReq) -> AnchorRes;
				/// Add or remove a descriptor classifier.
				async fn descriptor(req: DescriptorReq) -> DescriptorRes;
				/// Fire a stigmergic pulse through the root kern.
				async fn pulse(req: PulseReq) -> PulseRes;
				/// Generic MCP-style tool dispatch. Forwards the named tool's
				/// arguments to the daemon's `mcp::Server::call_tool` and
				/// returns the full MCP `{ content, isError? }` envelope. Used
				/// by the `kern mcp` proxy subprocess so it can relay stdio
				/// MCP `tools/call` requests over kern.sock without
				/// enumerating each tool as a typed method.
				async fn call_tool(req: CallToolReq) -> CallToolRes;
				/// Enumerate the daemon's live MCP tool schemas. Forwarded by the
				/// `kern mcp` proxy so a pane's `tools/list` reflects what the
				/// daemon actually exposes (e.g. the mux comms tools), not the
				/// proxy's static catalogue.
				async fn list_tools(req: ListToolsReq) -> ListToolsRes;
		}
}
