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
//! `fork_at` is **not** part of `KernRpc`. Forks are agnt's concern —
//! routing them through kern would force kern to know about agnt
//! sessions, which it deliberately doesn't. Slice L will introduce a
//! `fork_at` method on `AgntRpc` (see `protocol::agent`).

pub mod client_local;
pub mod dto;
pub mod mock;
pub mod svc;

pub use dto::{
    Anchor, AnchorReq, AnchorRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
    DescriptorRes, EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, ForgetReq, ForgetRes,
    HealthRes, IngestReq, IngestRes, LinkReq, LinkRes, NeighborsReq, NeighborsRes, PulseReq,
    PulseRes, QueryReq, QueryRes, SourceLite, TruncateAfterReq, TruncateAfterRes,
};
pub use mock::MockKernServer;
pub use svc::{serve_kern_rpc, KernRpc, KernRpcClient};
