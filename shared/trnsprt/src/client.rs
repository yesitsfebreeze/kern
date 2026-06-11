use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

use crate::error::McpError;
use crate::transport::Transport;
use crate::types::{ToolResult, ToolSchema};
use crate::PROTOCOL_VERSION;

/// Cap on consecutive frames whose `id` doesn't match the in-flight request
/// before [`Client::request`] gives up. Bounds the read loop so a peer flooding
/// unrelated frames (or a wire desync) can't spin forever; in normal use only a
/// handful of notifications are skipped before the matching reply arrives.
const MAX_UNMATCHED_FRAMES: usize = 1024;

/// Blocking MCP client over a newline-delimited JSON-RPC wire.
///
/// Each request is one `{jsonrpc, id, method, params}` object serialized to a
/// single line (no embedded newlines — [`Client::send`] rejects them) and
/// flushed; the matching reply is the line whose `id` equals the request's.
/// Fully SYNCHRONOUS: [`Client::request`] blocks reading frames off the transport
/// until it sees that id, so a `Client` has at most one in-flight request and is
/// not meant for concurrent calls from multiple threads. Notifications carry no
/// `id` and expect no reply. `rx_buf` retains bytes read past a frame boundary
/// for the next [`Client::recv`].
pub struct Client {
	transport: Box<dyn Transport>,
	next_id: AtomicU64,
	rx_buf: Vec<u8>,
}

impl Client {
	pub fn new(transport: Box<dyn Transport>) -> Self {
		Self {
			transport,
			next_id: AtomicU64::new(1),
			rx_buf: Vec::new(),
		}
	}

	pub fn initialize(&mut self, client_name: &str, client_version: &str) -> Result<Value, McpError> {
		let params = json!({
			"protocolVersion": PROTOCOL_VERSION,
			"capabilities": {},
			"clientInfo": { "name": client_name, "version": client_version },
		});
		let result = self.request("initialize", params)?;
		self.notify("notifications/initialized", json!({}))?;
		Ok(result)
	}

	pub fn list_tools(&mut self) -> Result<Vec<ToolSchema>, McpError> {
		let result = self.request("tools/list", json!({}))?;
		let tools = result
			.get("tools")
			.ok_or_else(|| McpError::Protocol("tools/list missing `tools`".into()))?
			.clone();
		let parsed: Vec<ToolSchema> = serde_json::from_value(tools)?;
		Ok(parsed)
	}

	pub fn call_tool(&mut self, name: &str, args: &Value) -> Result<ToolResult, McpError> {
		let params = json!({ "name": name, "arguments": args });
		let result = self.request("tools/call", params)?;
		let parsed: ToolResult = serde_json::from_value(result)?;
		Ok(parsed)
	}

	fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
		let id = self.next_id.fetch_add(1, Ordering::Relaxed);
		let msg = json!({
			"jsonrpc": "2.0",
			"id": id,
			"method": method,
			"params": params,
		});
		self.send(&msg)?;
		let mut skipped = 0usize;
		loop {
			let frame = self.recv()?;
			let matches = frame
				.get("id")
				.and_then(Value::as_u64)
				.map(|rid| rid == id)
				.unwrap_or(false);
			if !matches {
				skipped += 1;
				if skipped > MAX_UNMATCHED_FRAMES {
					return Err(McpError::Protocol(format!(
						"no reply for id {id} after {MAX_UNMATCHED_FRAMES} unmatched frames"
					)));
				}
				continue;
			}
			if let Some(err) = frame.get("error") {
				let code = err.get("code").and_then(Value::as_i64).unwrap_or(-1);
				let message = err
					.get("message")
					.and_then(Value::as_str)
					.unwrap_or("unknown")
					.to_string();
				return Err(McpError::Rpc { code, message });
			}
			return Ok(frame.get("result").cloned().unwrap_or(Value::Null));
		}
	}

	fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
		let msg = json!({
			"jsonrpc": "2.0",
			"method": method,
			"params": params,
		});
		self.send(&msg)
	}

	fn send(&mut self, msg: &Value) -> Result<(), McpError> {
		let mut line = serde_json::to_string(msg)?;
		if line.contains('\n') {
			return Err(McpError::Protocol("frame contained newline".into()));
		}
		line.push('\n');
		let w = self.transport.writer();
		w.write_all(line.as_bytes())?;
		w.flush()?;
		Ok(())
	}

	fn recv(&mut self) -> Result<Value, McpError> {
		let mut chunk = [0u8; 1024];
		loop {
			if let Some(pos) = self.rx_buf.iter().position(|&b| b == b'\n') {
				let line: Vec<u8> = self.rx_buf.drain(..=pos).collect();
				let line_str = std::str::from_utf8(&line[..pos])
					.map_err(|e| McpError::Protocol(format!("non-utf8 frame: {e}")))?;
				let trimmed = line_str.trim_end_matches('\r');
				let val: Value = serde_json::from_str(trimmed)?;
				return Ok(val);
			}
			let n = self.transport.reader().read(&mut chunk)?;
			if n == 0 {
				return Err(McpError::NotRunning);
			}
			self.rx_buf.extend_from_slice(&chunk[..n]);
		}
	}
}

impl Drop for Client {
	fn drop(&mut self) {
		let _ = self.transport.kill();
	}
}
