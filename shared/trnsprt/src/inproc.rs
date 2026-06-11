use std::io::{Read, Write};

use serde_json::Value;

use crate::server::{dispatch, McpServer};
use crate::transport::Transport;

/// In-process [`Transport`]: a synchronous loopback that runs an [`McpServer`]
/// directly in the caller's thread. Writing a newline-delimited JSON-RPC request
/// dispatches it immediately and buffers the reply for the next read — no socket,
/// no async, no separate process.
///
/// For TESTS and local-dev embedding ONLY. It is single-threaded and unbounded
/// (the request/response buffers grow with traffic), with no backpressure or
/// cancellation. Use the HTTP or local-socket transports for real deployments.
pub struct InProcTransport {
	server: Box<dyn McpServer>,
	req_buf: Vec<u8>,
	resp_buf: std::collections::VecDeque<u8>,
	killed: bool,
}

impl InProcTransport {
	pub fn new(server: Box<dyn McpServer>) -> Self {
		Self {
			server,
			req_buf: Vec::new(),
			resp_buf: std::collections::VecDeque::new(),
			killed: false,
		}
	}

	fn try_handle_one(&mut self) -> std::io::Result<()> {
		let pos = match self.req_buf.iter().position(|&b| b == b'\n') {
			Some(p) => p,
			None => return Ok(()),
		};
		let line: Vec<u8> = self.req_buf.drain(..=pos).collect();
		let line_str = match std::str::from_utf8(&line[..pos]) {
			Ok(s) => s.trim_end_matches('\r'),
			Err(_) => return Ok(()),
		};
		let req: Value = match serde_json::from_str(line_str) {
			Ok(v) => v,
			Err(_) => return Ok(()),
		};
		if let Some(frame) = dispatch(&*self.server, &req) {
			let mut s = serde_json::to_string(&frame).unwrap_or_default();
			s.push('\n');
			self.resp_buf.extend(s.as_bytes());
		}
		Ok(())
	}
}

impl Read for InProcTransport {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		if self.killed {
			return Ok(0);
		}
		// Bulk copy out of the ring buffer instead of popping byte-by-byte. The
		// VecDeque may straddle its internal wrap, so copy each contiguous slice in
		// turn, then drop the consumed prefix.
		let n = self.resp_buf.len().min(buf.len());
		if n == 0 {
			return Ok(0);
		}
		let (head, tail) = self.resp_buf.as_slices();
		let from_head = head.len().min(n);
		buf[..from_head].copy_from_slice(&head[..from_head]);
		if from_head < n {
			buf[from_head..n].copy_from_slice(&tail[..n - from_head]);
		}
		self.resp_buf.drain(..n);
		Ok(n)
	}
}

impl Write for InProcTransport {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		if self.killed {
			return Err(std::io::Error::new(
				std::io::ErrorKind::BrokenPipe,
				"inproc transport killed",
			));
		}
		self.req_buf.extend_from_slice(buf);
		while self.req_buf.contains(&b'\n') {
			self.try_handle_one()?;
		}
		Ok(buf.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

impl Transport for InProcTransport {
	fn reader(&mut self) -> &mut dyn Read {
		self
	}
	fn writer(&mut self) -> &mut dyn Write {
		self
	}
	fn kill(&mut self) -> std::io::Result<()> {
		self.killed = true;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	// Local mock over this crate's own McpServer (not test-utils::AdderServer):
	// trnsprt's dev-dep cycle (trnsprt -> test-utils -> trnsprt) makes a cross-crate
	// server impl a different trait instance in trnsprt's own unit-test build.
	struct EchoServer;
	impl McpServer for EchoServer {
		fn tools_list(&self) -> Vec<crate::ToolSchema> {
			vec![crate::ToolSchema { name: "echo".into(), description: None, input_schema: None }]
		}
		fn call_tool(&self, name: &str, args: &Value) -> Result<crate::ToolResult, crate::McpError> {
			Ok(crate::ToolResult {
				content: vec![json!({ "tool": name, "args": args.clone() })],
				is_error: false,
				structured_content: None,
			})
		}
	}

	fn write_line(t: &mut InProcTransport, v: &Value) {
		let mut s = serde_json::to_string(v).unwrap();
		s.push('\n');
		t.write_all(s.as_bytes()).unwrap();
	}

	#[test]
	fn request_round_trips_to_a_newline_terminated_response() {
		let mut t = InProcTransport::new(Box::new(EchoServer));
		write_line(&mut t, &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));

		let mut buf = [0u8; 256];
		let n = t.read(&mut buf).unwrap();
		assert!(n > 0, "a response is buffered after the write");
		let line = std::str::from_utf8(&buf[..n]).unwrap();
		assert!(line.ends_with('\n'), "frames are newline-terminated");
		let resp: Value = serde_json::from_str(line.trim()).unwrap();
		assert_eq!(resp["id"], 1, "id echoed");
		assert!(resp["result"]["tools"].is_array());
		assert!(serde_json::to_string(&resp).unwrap().contains("echo"));
	}

	#[test]
	fn tiny_reads_reassemble_the_full_frame_across_the_ring() {
		// A 4-byte buffer forces many reads, exercising the bulk-copy path and any
		// wrap in the response ring buffer.
		let mut t = InProcTransport::new(Box::new(EchoServer));
		write_line(&mut t, &json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/list" }));
		let mut out = Vec::new();
		let mut chunk = [0u8; 4];
		loop {
			let n = t.read(&mut chunk).unwrap();
			if n == 0 {
				break;
			}
			out.extend_from_slice(&chunk[..n]);
		}
		let resp: Value = serde_json::from_str(std::str::from_utf8(&out).unwrap().trim()).unwrap();
		assert_eq!(resp["id"], 9);
	}

	#[test]
	fn read_returns_eof_after_kill() {
		let mut t = InProcTransport::new(Box::new(EchoServer));
		write_line(&mut t, &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
		t.kill().unwrap();
		let mut buf = [0u8; 16];
		assert_eq!(t.read(&mut buf).unwrap(), 0, "a killed transport reads as EOF");
	}

	#[test]
	fn write_after_kill_is_a_broken_pipe() {
		let mut t = InProcTransport::new(Box::new(EchoServer));
		t.kill().unwrap();
		let err = t.write(b"{}\n").unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
	}
}
