//! `KernRpc` — typed-RPC surface exposing kern's read+write operations
//! to sub-agents and a client.
//!
//!
//! Layout:
//! - [`dto`] — wire types ([`QueryReq`], [`IngestReq`], ...). Several
//!   are re-exported from [`SearchSvc`](crate::search) so the two
//!   services share a wire vocabulary.
//! - [`svc`] — `service!` invocation that emits [`KernRpc`],
//!   [`KernRpcClient`], and [`serve_kern_rpc`].
//! - [`mock`] — in-memory [`MockKernServer`] for tests and downstream
//!   slice development.
//! - [`client_local`] — convenience constructor that dials the per-user
//!   `kern.sock` endpoint and builds a `KernRpcClient`.
//!
//! Forks are deliberately **not** part of `KernRpc` — routing agent
//! session forks through kern would force it to know about agent
//! sessions, which it doesn't.

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
