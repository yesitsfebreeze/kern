//! Typed-RPC server modules.
//!
//! Implements the `trnsprt::kern_rpc` surface — the typed read+write API
//! consumed by sub-agents and external MCP clients. Bound to the per-user
//! `kern.sock` singleton endpoint via `trnsprt::typed::LocalListener`.

pub mod kern_rpc_server;

pub use kern_rpc_server::{serve_kern_rpc_loop, KernRpcHandler};
