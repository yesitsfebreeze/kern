mod error;
mod http;
mod server;
mod types;

pub use error::McpError;
pub use http::serve_http;
pub use server::{serve_rw, serve_stdio, McpServer};
pub use types::{ToolResult, ToolSchema};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

// `service!` emits `::trnsprt::*` paths; the self-alias makes them resolve
// when the macro is invoked inside this crate.
extern crate self as trnsprt;

pub mod typed;
pub use trnsprt_macros::service;

pub mod hub_rpc;
pub mod kern_rpc;

// Re-exports solely for service!-generated code (::trnsprt::__private::*).
// NOT public API — may change in any release; never import directly.
#[doc(hidden)]
pub mod __private {
	pub use bytes;
	pub use futures;
	pub use serde_json;
	pub use tokio;
	pub use tokio_util;
}
