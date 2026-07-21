pub mod prompt;
pub mod resources;
pub mod sse;
pub mod tools;
mod tools_admin;
mod tools_mutate;
mod tools_query;
mod tools_setup;

use std::io::{BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use parking_lot::RwLock;

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

pub type PulseBroadcast = Arc<dyn Fn(&str, f64) + Send + Sync>;

pub struct Server {
	pub graph: Arc<RwLock<GraphGnn>>,
	pub worker: Arc<ingest::Worker>,
	pub llm: Option<llm::Client>,
	pub save_fn: Arc<dyn Fn() + Send + Sync>,
	pub task_q: Option<Arc<tick::queue::Queue>>,
	pub cfg: Arc<Config>,
	pub cache: Arc<Mutex<QueryCache>>,
	pub broadcast_pulse: Option<PulseBroadcast>,
	// Epoch ms of the last real tool call (health polls excluded, or the hub's
	// own idle probe would keep every node alive forever). Seeded at boot so a
	// never-used node counts idle from startup.
	pub last_activity: Arc<std::sync::atomic::AtomicU64>,
}

impl Server {
	pub fn idle_ms(&self) -> u64 {
		let last = self
			.last_activity
			.load(std::sync::atomic::Ordering::Relaxed);
		crate::base::util::now_ms().saturating_sub(last)
	}

	pub(crate) fn touch(&self) {
		self.last_activity.store(
			crate::base::util::now_ms(),
			std::sync::atomic::Ordering::Relaxed,
		);
	}
}

#[derive(Default)]
struct TickHealth {
	queue_depth: u64,
	tasks_done: u64,
	task_avg_ms: u64,
	task_panics: u64,
	last_task_panic: Option<String>,
	task_failures: u64,
	last_task_failure: Option<String>,
}

impl TickHealth {
	fn of(q: &Arc<tick::queue::Queue>) -> Self {
		let (done, avg_ms) = q.metrics();
		let (task_panics, last_panic) = q.panics();
		let (task_failures, last_failure) = q.failures();
		Self {
			queue_depth: q.pending_count() as u64,
			tasks_done: done.max(0) as u64,
			task_avg_ms: avg_ms.max(0) as u64,
			task_panics,
			last_task_panic: last_panic.map(|p| p.to_string()),
			task_failures,
			last_task_failure: last_failure.map(|f| f.to_string()),
		}
	}
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
		let g = self.graph.read();
		let h = crate::base::health::graph_health_stats(&g);
		let descriptors = g.root.descriptors.len();
		let tick = self.task_q.as_ref().map(TickHealth::of).unwrap_or_default();
		serde_json::json!({
			"gravitons": h.gravitons,
			"kerns": h.kerns,
			"entities": h.entities,
			"reasons": h.reasons,
			"unnamed": h.unnamed,
			"descriptors": descriptors,
			"queue_depth": tick.queue_depth,
			"tasks_done": tick.tasks_done,
			"task_avg_ms": tick.task_avg_ms,
			"task_panics": tick.task_panics,
			"last_task_panic": tick.last_task_panic,
			"task_failures": tick.task_failures,
			"last_task_failure": tick.last_task_failure,
			"cold_evicted": h.cold_evicted,
			"embed_model": h.embed_model,
			"embed_dim": h.embed_dim,
			"embed_mismatch": h.embed_mismatch,
		})
	}
}

impl trnsprt::McpServer for Server {
	fn server_name(&self) -> &str {
		"kern"
	}
	fn server_version(&self) -> &str {
		env!("CARGO_PKG_VERSION")
	}

	fn extra_capabilities(&self) -> serde_json::Value {
		serde_json::json!({"resources": {}, "prompts": {}})
	}

	fn tools_list(&self) -> Vec<trnsprt::ToolSchema> {
		tools::typed_tool_schemas()
	}

	fn call_tool(
		&self,
		name: &str,
		args: &serde_json::Value,
	) -> Result<trnsprt::ToolResult, trnsprt::McpError> {
		if name != "health" {
			self.touch();
		}
		let result = match name {
			"query" => self.tool_query(args),
			"ingest" => self.tool_ingest(args),
			"link" => self.tool_link(args),
			"forget" => self.tool_forget(args),
			"degrade" => self.tool_degrade(args),
			"move" => self.tool_move(args),
			"health" => self.tool_health(),
			"graviton" => self.tool_graviton(args),
			"descriptor" => self.tool_descriptor(args),
			"pulse" => self.tool_pulse(args),
			"gc" => self.tool_gc(),
			"setup" => self.tool_setup(),
			_ => {
				return Ok(trnsprt::ToolResult {
					content: vec![
						serde_json::json!({"type": "text", "text": format!("unknown tool: {name}")}),
					],
					is_error: true,
					structured_content: None,
				})
			}
		};
		Ok(value_to_tool_result(&result))
	}

	fn handle_method(
		&self,
		method: &str,
		params: serde_json::Value,
	) -> Option<Result<serde_json::Value, trnsprt::McpError>> {
		let raw = serde_json::value::RawValue::from_string(
			serde_json::to_string(&params).unwrap_or_else(|_| "null".to_string()),
		)
		.ok();
		match method {
			"resources/list" => Some(Ok(
				serde_json::json!({"resources": resources::resource_definitions()}),
			)),
			"resources/read" => Some(response_to_result(resources::handle_resource_read(
				self, None, raw,
			))),
			"prompts/list" => Some(Ok(
				serde_json::json!({"prompts": prompt::prompt_definitions()}),
			)),
			"prompts/get" => Some(response_to_result(prompt::handle_prompt_get(None, raw))),
			"ping" => Some(Ok(serde_json::json!({}))),
			_ => None,
		}
	}
}

pub(crate) fn value_to_tool_result(v: &serde_json::Value) -> trnsprt::ToolResult {
	let is_error = v
		.get("isError")
		.and_then(serde_json::Value::as_bool)
		.unwrap_or(false);
	let content = v
		.get("content")
		.and_then(serde_json::Value::as_array)
		.cloned()
		.unwrap_or_default();
	trnsprt::ToolResult {
		content,
		is_error,
		structured_content: None,
	}
}

fn response_to_result(resp: Response) -> Result<serde_json::Value, trnsprt::McpError> {
	match (resp.result, resp.error) {
		(Some(v), _) => Ok(v),
		(None, Some(e)) => Err(trnsprt::McpError::Rpc {
			code: e.code as i64,
			message: e.message,
		}),
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

#[cfg(test)]
mod tests {
	use serde_json::json;

	#[test]
	fn envelope_extracts_content_array_and_error_flag() {
		let env = json!({ "content": [{ "type": "text", "text": "hi" }], "isError": true });
		let r = super::value_to_tool_result(&env);
		assert!(r.is_error);
		assert_eq!(r.content.len(), 1);
		assert_eq!(r.content[0]["text"], "hi");
		assert!(r.structured_content.is_none());
	}

	#[test]
	fn envelope_missing_fields_default_to_empty_and_ok() {
		let r = super::value_to_tool_result(&json!({}));
		assert!(!r.is_error, "missing isError defaults to false");
		assert!(r.content.is_empty(), "missing content defaults to empty");
	}

	#[test]
	fn envelope_non_array_content_falls_back_to_empty() {
		let r = super::value_to_tool_result(&json!({ "content": "oops", "isError": false }));
		assert!(
			r.content.is_empty(),
			"a non-array content is ignored, not panicked on"
		);
	}

	#[tokio::test]
	async fn health_reports_degraded_maintenance_after_a_task_panic() {
		use crate::tick::queue::{task, Queue, TaskKind};
		use std::sync::Arc;

		let mut srv = crate::test_support::mcp_server();
		let q = Arc::new(Queue::new(8));
		srv.task_q = Some(q.clone());

		let h = srv.health_stats();
		assert_eq!(h["task_panics"], 0);
		assert!(h["last_task_panic"].is_null(), "healthy queue reports null");

		q.record_task_panic(&task(TaskKind::GnnPropagate, "k1"), "adj*features");

		let h = srv.health_stats();
		assert_eq!(h["task_panics"], 1);
		assert_eq!(h["last_task_panic"], "GnnPropagate[k1]: adj*features");
	}

	#[tokio::test]
	async fn health_reports_contained_task_failures_beside_panics() {
		use crate::tick::queue::{task, Queue, TaskKind};
		use std::sync::Arc;

		let mut srv = crate::test_support::mcp_server();
		let q = Arc::new(Queue::new(8));
		srv.task_q = Some(q.clone());

		let h = srv.health_stats();
		assert_eq!(h["task_failures"], 0);
		assert!(h["last_task_failure"].is_null());

		q.record_task_failure(
			&task(TaskKind::GnnPropagate, "k1"),
			"could not sample negative edges",
		);

		let h = srv.health_stats();
		assert_eq!(h["task_failures"], 1);
		assert_eq!(
			h["last_task_failure"],
			"GnnPropagate[k1]: could not sample negative edges"
		);
		assert_eq!(h["task_panics"], 0, "a failure is not a panic");
	}

	#[tokio::test]
	async fn health_carries_the_store_signals_to_the_mcp_surface() {
		let srv = crate::test_support::mcp_server();
		let h = srv.health_stats();
		for key in ["cold_evicted", "embed_model", "embed_dim", "embed_mismatch"] {
			assert!(!h[key].is_null(), "{key} must reach the MCP surface");
		}
		assert_eq!(h["cold_evicted"], 0);
		assert_eq!(h["embed_mismatch"], false);
	}
}
