//! `KernRpc` service definition — the `service!` macro generates the trait,
//! `KernRpcClient<C>`, and the `serve_kern_rpc(channel, handler)` loop.

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
				/// Generic MCP tool dispatch via the daemon's `mcp::Server::call_tool`;
				/// returns the full MCP `{ content, isError? }` envelope.
				async fn call_tool(req: CallToolReq) -> CallToolRes;
				/// Enumerate the daemon's live MCP tool schemas.
				async fn list_tools(req: ListToolsReq) -> ListToolsRes;
		}
}
