use std::sync::Arc;

use super::Server;

// Despite the module name `sse`, this is MCP Streamable HTTP, not a WebSocket.
pub async fn run_sse(server: Arc<Server>, addr: &str) -> Result<(), std::io::Error> {
	trnsprt::serve_http(server, addr).await
}
