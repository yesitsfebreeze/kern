//! Typed-RPC server half of `trnsprt::kern_rpc`, bound to the per-user
//! `kern.sock` singleton. Import the re-exports here, not `kern_rpc_server`.

pub mod kern_rpc_server;

pub use kern_rpc_server::{serve_kern_rpc_loop, KernRpcHandler};
