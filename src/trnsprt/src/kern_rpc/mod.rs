pub mod client_local;
pub mod dto;
pub mod svc;

pub use dto::{CallToolReq, CallToolRes, HealthRes, ListToolsReq, ListToolsRes, ShutdownRes};
pub use svc::{serve_kern_rpc, KernRpc, KernRpcClient};
