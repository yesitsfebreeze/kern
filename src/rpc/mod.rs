//! Typed-RPC server modules.
//!
//! Implements the `trnsprt::kern_rpc` surface — the typed read+write API
//! consumed by sub-agents and external MCP clients. Bound to the per-user
//! `kern.sock` singleton endpoint via `trnsprt::typed::LocalListener`.
//!
//! This is the SERVER half of the boundary (kern implements the `KernRpc` trait);
//! clients dial in with `trnsprt::kern_rpc::KernRpcClient`. Consumers of this
//! module import the re-exported [`serve_kern_rpc_loop`] (the accept/serve driver)
//! and [`KernRpcHandler`] (kern's trait impl) — not `kern_rpc_server` directly.

pub mod kern_rpc_server;

pub use kern_rpc_server::{serve_kern_rpc_loop, KernRpcHandler};
