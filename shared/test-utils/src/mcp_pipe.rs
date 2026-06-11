use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use trnsprt::{McpError, McpServer, ToolResult, ToolSchema, Transport};

#[derive(Default)]
struct Wire {
	to_server: Vec<u8>,
	from_server: Vec<u8>,
	server_pos: usize,
	killed: bool,
}

/// One client-side end of the in-memory pipe, with a deliberate directional
/// asymmetry: its `Read` impl drains `from_server` (server → client), while its
/// `Write` impl appends to `to_server` (client → server). `PipeTransport` holds
/// two `ClientEnd`s over the SAME [`Wire`] and uses only each one's matching half
/// — `reader` for [`Read`], `writer` for [`Write`]. The split is purely which
/// trait method gets called; a `ClientEnd` driven through the opposite trait
/// would silently touch the wrong buffer.
struct ClientEnd(Arc<Mutex<Wire>>);

impl Read for ClientEnd {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		let mut w = self.0.lock().expect("lock");
		let available = &w.from_server[w.server_pos..];
		let n = available.len().min(buf.len());
		buf[..n].copy_from_slice(&available[..n]);
		w.server_pos += n;
		Ok(n)
	}
}

impl Write for ClientEnd {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		let mut w = self.0.lock().expect("lock");
		w.to_server.extend_from_slice(buf);
		Ok(buf.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

pub struct PipeTransport {
	reader: ClientEnd,
	writer: ClientEnd,
	wire: Arc<Mutex<Wire>>,
}

impl Transport for PipeTransport {
	fn reader(&mut self) -> &mut dyn Read {
		&mut self.reader
	}
	fn writer(&mut self) -> &mut dyn Write {
		&mut self.writer
	}
	fn kill(&mut self) -> std::io::Result<()> {
		self.wire.lock().expect("lock").killed = true;
		Ok(())
	}
}

pub struct PipeHandle(Arc<Mutex<Wire>>);

impl PipeHandle {
	pub fn drain_frames(&self) -> Vec<Value> {
		let mut w = self.0.lock().expect("lock");
		let bytes = std::mem::take(&mut w.to_server);
		let s = String::from_utf8(bytes).expect("utf8");
		s.lines()
			.filter(|l| !l.is_empty())
			.map(|l| serde_json::from_str(l).expect("json"))
			.collect()
	}

	pub fn push_reply(&self, msg: &Value) {
		let mut w = self.0.lock().expect("lock");
		let mut s = serde_json::to_string(msg).expect("json");
		s.push('\n');
		w.from_server.extend_from_slice(s.as_bytes());
	}

	pub fn killed(&self) -> bool {
		self.0.lock().expect("lock").killed
	}
}

pub fn new_pipe() -> (PipeTransport, PipeHandle) {
	let wire = Arc::new(Mutex::new(Wire::default()));
	let t = PipeTransport {
		reader: ClientEnd(wire.clone()),
		writer: ClientEnd(wire.clone()),
		wire: wire.clone(),
	};
	(t, PipeHandle(wire))
}

pub fn reply_result(id: u64, result: Value) -> Value {
	json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn reply_error(id: u64, code: i64, message: &str) -> Value {
	json!({
		"jsonrpc": "2.0",
		"id": id,
		"error": { "code": code, "message": message },
	})
}

pub struct AdderServer;

impl McpServer for AdderServer {
	fn tools_list(&self) -> Vec<ToolSchema> {
		vec![ToolSchema {
			name: "add".into(),
			description: Some("a+b".into()),
			input_schema: None,
		}]
	}
	fn call_tool(&self, name: &str, args: &Value) -> Result<ToolResult, McpError> {
		if name != "add" {
			return Err(McpError::Rpc {
				code: -32601,
				message: format!("unknown tool: {name}"),
			});
		}
		// Strict params: both operands are required and must be integers. A
		// missing or non-integer arg is a -32602 (Invalid params) Rpc error
		// rather than a silent default-to-zero, so callers can exercise the
		// argument-validation error path.
		let a = args.get("a").and_then(Value::as_i64).ok_or_else(|| McpError::Rpc {
			code: -32602,
			message: "missing or non-integer argument: a".into(),
		})?;
		let b = args.get("b").and_then(Value::as_i64).ok_or_else(|| McpError::Rpc {
			code: -32602,
			message: "missing or non-integer argument: b".into(),
		})?;
		Ok(ToolResult {
			content: vec![json!({ "type": "text", "text": (a + b).to_string() })],
			is_error: false,
			structured_content: None,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn push_reply_is_readable_through_the_transport_reader() {
		// Server -> client direction: a reply pushed via the handle shows up on the
		// transport's Read half.
		let (mut transport, handle) = new_pipe();
		handle.push_reply(&reply_result(1, json!({ "ok": true })));

		let mut buf = [0u8; 256];
		let n = transport.reader().read(&mut buf).expect("read");
		let line = std::str::from_utf8(&buf[..n]).expect("utf8");
		let v: Value = serde_json::from_str(line.trim_end()).expect("json");
		assert_eq!(v["id"], 1);
		assert_eq!(v["result"]["ok"], true);
	}

	#[test]
	fn writes_through_the_transport_writer_are_drained_as_frames() {
		// Client -> server direction: bytes written to the transport's Write half
		// are recovered as parsed JSON frames by the handle.
		let (mut transport, handle) = new_pipe();
		let mut line = serde_json::to_string(&json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" })).unwrap();
		line.push('\n');
		transport.writer().write_all(line.as_bytes()).expect("write");

		let frames = handle.drain_frames();
		assert_eq!(frames.len(), 1);
		assert_eq!(frames[0]["id"], 7);
		assert_eq!(frames[0]["method"], "ping");
		assert!(handle.drain_frames().is_empty(), "draining consumes the buffer");
	}

	#[test]
	fn kill_flips_the_shared_killed_flag() {
		let (mut transport, handle) = new_pipe();
		assert!(!handle.killed());
		transport.kill().expect("kill");
		assert!(handle.killed(), "kill is observable through the handle");
	}
}
