use std::collections::hash_map::Entry;
use std::collections::HashMap;

use serde_json::Value;

use crate::client::Client;
use crate::error::McpError;
use crate::inproc::InProcTransport;
use crate::server::McpServer;
use crate::transport::ChildStdio;
use crate::types::{ServerId, ToolResult, ToolSchema};

/// A connected MCP server: the [`Client`] that drives its transport plus the
/// tool schema snapshot taken at connect time. The snapshot is what
/// [`Registry::list_tools`] serves without a round-trip; call
/// [`refresh_tools`](LiveServer::refresh_tools) to re-pull it after the server's
/// tool set is known to have changed (schemas otherwise go stale silently).
pub struct LiveServer {
	pub(crate) client: Client,
	pub(crate) tools: Vec<ToolSchema>,
}

impl LiveServer {
	pub fn new(client: Client, tools: Vec<ToolSchema>) -> Self {
		Self { client, tools }
	}

	pub fn tools(&self) -> &[ToolSchema] {
		&self.tools
	}

	pub fn refresh_tools(&mut self) -> Result<&[ToolSchema], McpError> {
		self.tools = self.client.list_tools()?;
		Ok(&self.tools)
	}

	pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolResult, McpError> {
		self.client.call_tool(name, args)
	}
}

/// Owns every connected MCP server keyed by [`ServerId`] and routes tool calls to
/// the right one. The `Registry` is the lifecycle owner: registering
/// (`spawn_stdio` / `register_inproc`) performs the MCP `initialize` handshake and
/// caches the server's tool schemas; the cache lives until `refresh_tools` re-pulls
/// it or `remove` drops the server. Registration is idempotent-safe — a duplicate
/// [`ServerId`] is rejected with [`McpError::DuplicateServer`] rather than
/// silently replacing the live connection.
#[derive(Default)]
pub struct Registry {
	servers: HashMap<ServerId, LiveServer>,
}

impl Registry {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn insert(&mut self, id: ServerId, server: LiveServer) {
		self.servers.insert(id, server);
	}

	pub fn spawn_stdio(
		&mut self,
		id: ServerId,
		program: &str,
		args: &[&str],
	) -> Result<&LiveServer, McpError> {
		let transport = ChildStdio::spawn(program, args)?;
		self.install(id, Box::new(transport))
	}

	pub fn register_inproc(
		&mut self,
		id: ServerId,
		server: Box<dyn McpServer>,
	) -> Result<&LiveServer, McpError> {
		let transport = InProcTransport::new(server);
		self.install(id, Box::new(transport))
	}

	fn install(
		&mut self,
		id: ServerId,
		transport: Box<dyn crate::transport::Transport>,
	) -> Result<&LiveServer, McpError> {
		let mut client = Client::new(transport);
		client.initialize("kern", env!("CARGO_PKG_VERSION"))?;
		let tools = client.list_tools()?;
		match self.servers.entry(id) {
			Entry::Vacant(e) => Ok(e.insert(LiveServer { client, tools })),
			Entry::Occupied(e) => Err(McpError::DuplicateServer(e.key().0.clone())),
		}
	}

	pub fn server_ids(&self) -> Vec<ServerId> {
		self.servers.keys().cloned().collect()
	}

	pub fn list_tools(&self, id: &ServerId) -> Result<&[ToolSchema], McpError> {
		self.servers
			.get(id)
			.map(|s| s.tools.as_slice())
			.ok_or_else(|| McpError::UnknownServer(id.0.clone()))
	}

	pub fn call_tool(
		&mut self,
		id: &ServerId,
		name: &str,
		args: &Value,
	) -> Result<ToolResult, McpError> {
		let server = self
			.servers
			.get_mut(id)
			.ok_or_else(|| McpError::UnknownServer(id.0.clone()))?;
		server.call_tool(name, args)
	}

	pub fn remove(&mut self, id: &ServerId) {
		self.servers.remove(id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	/// Minimal in-process MCP server: one `echo` tool that returns its args.
	struct MockServer;

	impl McpServer for MockServer {
		fn tools_list(&self) -> Vec<ToolSchema> {
			vec![ToolSchema {
				name: "echo".into(),
				description: Some("echoes its arguments back".into()),
				input_schema: Some(json!({ "type": "object" })),
			}]
		}

		fn call_tool(&self, name: &str, args: &Value) -> Result<ToolResult, McpError> {
			match name {
				"echo" => Ok(ToolResult {
					content: vec![args.clone()],
					is_error: false,
					structured_content: None,
				}),
				other => Err(McpError::UnknownServer(other.to_string())),
			}
		}
	}

	#[test]
	fn register_inproc_seeds_tools_and_routes_calls() {
		let mut reg = Registry::new();
		let id = ServerId("mock".to_string());
		reg.register_inproc(id.clone(), Box::new(MockServer))
			.expect("registration performs the initialize handshake");

		// The schema snapshot taken at connect time is served without a round-trip.
		let tools = reg.list_tools(&id).expect("known server");
		assert_eq!(tools.len(), 1);
		assert_eq!(tools[0].name, "echo");

		// A call routes through the in-process transport + client and back.
		let out = reg
			.call_tool(&id, "echo", &json!({ "x": 1 }))
			.expect("echo call succeeds");
		assert!(!out.is_error);
		assert_eq!(out.content, vec![json!({ "x": 1 })]);
	}

	#[test]
	fn duplicate_server_id_is_rejected() {
		let mut reg = Registry::new();
		let id = ServerId("dup".to_string());
		reg.register_inproc(id.clone(), Box::new(MockServer)).unwrap();
		let again = reg.register_inproc(id.clone(), Box::new(MockServer));
		assert!(matches!(again, Err(McpError::DuplicateServer(_))));
	}

	#[test]
	fn list_tools_for_unknown_server_errors() {
		let reg = Registry::new();
		let err = reg.list_tools(&ServerId("nope".to_string()));
		assert!(matches!(err, Err(McpError::UnknownServer(_))));
	}
}
