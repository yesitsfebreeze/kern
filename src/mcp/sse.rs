//! MCP-over-HTTP transport entry point. The actual HTTP server lives in
//! `trnsprt`; this is just the kern-side adapter.

use std::sync::Arc;

use super::Server;

/// Serve the MCP [`Server`] over HTTP at `addr`. Despite the module name `sse`,
/// this is the 2025 MCP **Streamable HTTP** transport, not a bare WebSocket.
pub async fn run_sse(server: Arc<Server>, addr: &str) -> Result<(), std::io::Error> {
	trnsprt::serve_http(server, addr).await
}
