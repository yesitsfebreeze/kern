pub mod prompt;
pub mod resources;
pub mod sse;
pub mod tools;
mod tools_admin;
mod tools_mutate;
mod tools_query;

use std::io::{BufReader, Read, Write};
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use serde_json::value::RawValue;

use crate::base::graph::GraphGnn;
use crate::config::Config;
use crate::ingest;
use crate::llm;
use crate::retrieval::cache::QueryCache;
use crate::tick;

#[derive(Serialize)]
pub(crate) struct Response {
	jsonrpc: &'static str,
	#[serde(skip_serializing_if = "Option::is_none")]
	id: Option<Box<RawValue>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	result: Option<serde_json::Value>,
	#[serde(skip_serializing_if = "Option::is_none")]
	error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
	code: i32,
	message: String,
}

pub(crate) const ERR_INVALID_REQ: i32 = -32600;
pub(crate) const ERR_NOT_FOUND: i32 = -32601;

pub struct Server {
	pub graph: Arc<RwLock<GraphGnn>>,
	pub worker: Arc<ingest::Worker>,
	pub llm: Option<llm::Client>,
	pub save_fn: Arc<dyn Fn() + Send + Sync>,
	pub task_q: Option<Arc<tick::queue::Queue>>,
	pub cfg: Arc<Config>,
	/// Semantic cache over answered queries. Shared, so it is wrapped in a
	/// `Mutex`; lookups/inserts are brief (a linear scan of a small bounded
	/// ring). See [`crate::retrieval::cache`].
	pub cache: Arc<Mutex<QueryCache>>,
	/// Present only when this engine is hosted inside the mux TUI process.
	/// `Some` → the comms tools (`delegate`/`collect`/`spawn`/`send`/`panes`/
	/// `status`) are advertised and dispatched against this live pane registry.
	/// `None` → headless daemon; comms tools are absent.
	pub mux: Option<Arc<Mutex<crate::mux::registry::PaneRegistry>>>,
}

impl Server {
	pub fn run(&self, input: impl Read, output: impl Write) {
		let mut reader = BufReader::with_capacity(1024 * 1024, input);
		let mut output = output;
		let _ = trnsprt::serve_rw(&mut reader, &mut output, self);
	}

	pub fn run_stdio(&self) {
		let _ = trnsprt::serve_stdio(self);
	}

	pub(crate) fn health_stats(&self) -> serde_json::Value {
		let g = crate::base::locks::read_recovered(&self.graph);
		let h = crate::base::health::graph_health_stats(&g);
		let descriptors = g.root.descriptors.len();
		serde_json::json!({
			"anchors": h.anchors,
			"kerns": h.kerns,
			"entities": h.entities,
			"reasons": h.reasons,
			"unnamed": h.unnamed,
			"descriptors": descriptors,
		})
	}
}

impl trnsprt::McpServer for Server {
	fn server_name(&self) -> &str { "kern" }
	fn server_version(&self) -> &str { env!("CARGO_PKG_VERSION") }

	fn extra_capabilities(&self) -> serde_json::Value {
		serde_json::json!({"resources": {}, "prompts": {}})
	}

	fn tools_list(&self) -> Vec<trnsprt::ToolSchema> {
		tools::tool_definitions()
			.into_iter()
			.filter_map(|v| serde_json::from_value(v).ok())
			.collect()
	}

	fn call_tool(&self, name: &str, args: &serde_json::Value) -> Result<trnsprt::ToolResult, trnsprt::McpError> {
		let result = match name {
			"query"      => self.tool_query(args),
			"ingest"     => self.tool_ingest(args),
			"link"       => self.tool_link(args),
			"forget"     => self.tool_forget(args),
			"degrade"    => self.tool_degrade(args),
			"health"     => self.tool_health(),
			"anchor"     => self.tool_anchor(args),
			"descriptor" => self.tool_descriptor(args),
			"pulse"      => self.tool_pulse(args),
			_ => return Ok(trnsprt::ToolResult {
				content: vec![serde_json::json!({"type": "text", "text": format!("unknown tool: {name}")})],
				is_error: true,
				structured_content: None,
			}),
		};
		Ok(value_to_tool_result(result))
	}

	fn handle_method(&self, method: &str, params: serde_json::Value) -> Option<Result<serde_json::Value, trnsprt::McpError>> {
		let raw = serde_json::value::RawValue::from_string(
			serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string()),
		).ok();
		match method {
			"resources/list" => Some(Ok(serde_json::json!({"resources": resources::resource_definitions()}))),
			"resources/read" => Some(response_to_result(resources::handle_resource_read(self, None, raw))),
			"prompts/list"   => Some(Ok(serde_json::json!({"prompts": prompt::prompt_definitions()}))),
			"prompts/get"    => Some(response_to_result(prompt::handle_prompt_get(None, raw))),
			"ping"           => Some(Ok(serde_json::json!({}))),
			_                => None,
		}
	}
}

fn value_to_tool_result(v: serde_json::Value) -> trnsprt::ToolResult {
	let is_error = v.get("isError").and_then(serde_json::Value::as_bool).unwrap_or(false);
	let content = v.get("content")
		.and_then(serde_json::Value::as_array)
		.cloned()
		.unwrap_or_default();
	trnsprt::ToolResult { content, is_error, structured_content: None }
}

fn response_to_result(resp: Response) -> Result<serde_json::Value, trnsprt::McpError> {
	match (resp.result, resp.error) {
		(Some(v), _) => Ok(v),
		(None, Some(e)) => Err(trnsprt::McpError::Rpc { code: e.code as i64, message: e.message }),
		(None, None) => Ok(serde_json::Value::Null),
	}
}

pub(crate) fn ok(id: Option<Box<RawValue>>, result: serde_json::Value) -> Response {
	Response {
		jsonrpc: "2.0",
		id,
		result: Some(result),
		error: None,
	}
}

pub(crate) fn err_resp(id: Option<Box<RawValue>>, code: i32, msg: &str) -> Response {
	Response {
		jsonrpc: "2.0",
		id,
		result: None,
		error: Some(RpcError {
			code,
			message: msg.to_string(),
		}),
	}
}

fn tool_result(content: &str) -> serde_json::Value {
	serde_json::json!({
		"content": [{"type": "text", "text": content}],
	})
}

pub(crate) fn tool_result_json(v: &serde_json::Value) -> serde_json::Value {
	let s = serde_json::to_string(v).unwrap_or_default();
	tool_result(&s)
}

pub(crate) fn tool_error(msg: &str) -> serde_json::Value {
	serde_json::json!({
		"isError": true,
		"content": [{"type": "text", "text": msg}],
	})
}

