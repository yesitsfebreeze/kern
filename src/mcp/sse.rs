//! MCP-over-HTTP transport entry point.
//!
//! Kept as its own tiny module — rather than inlined into `mcp.rs` — so the
//! transport wiring stays separate from the [`Server`] / tool-dispatch logic.
//! The actual HTTP server lives in `trnsprt`; this is just the kern-side adapter.

use std::sync::Arc;

use super::Server;

/// Serve the MCP [`Server`] over HTTP at `addr` by delegating to
/// [`trnsprt::serve_http`]. Despite the module name `sse`, this is the 2025 MCP
/// **Streamable HTTP** transport (POST `/mcp` for request→response, GET `/mcp`
/// for the server-sent-events notification stream) — not a bare WebSocket.
/// Returns when the listener errors; the error is propagated to the caller, which
/// logs it (see `commands.rs`).
pub async fn run_sse(server: Arc<Server>, addr: &str) -> Result<(), std::io::Error> {
	trnsprt::serve_http(server, addr).await
}
