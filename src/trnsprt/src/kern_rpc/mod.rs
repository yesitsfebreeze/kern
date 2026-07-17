//! `KernRpc` — typed-RPC surface exposing kern's read+write operations
//! to sub-agents and other clients.

pub mod client_local;
pub mod dto;
pub mod mock;
pub mod svc;

pub use dto::{
	Anchor, AnchorReq, AnchorRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
	DescriptorRes, EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, ForgetReq, ForgetRes,
	HealthRes, IngestReq, IngestRes, LinkReq, LinkRes, ListToolsReq, ListToolsRes, NeighborsReq,
	NeighborsRes, PulseReq, PulseRes, QueryReq, QueryRes, SourceLite,
};
pub use mock::MockKernServer;
pub use svc::{serve_kern_rpc, KernRpc, KernRpcClient};
