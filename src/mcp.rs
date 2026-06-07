pub mod prompt;
pub mod resources;
pub mod sse;
pub mod tools;
mod tools_admin;
mod tools_mutate;
mod tools_query;

use std::io::{BufReader, Read, Write};
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::value::RawValue;

use crate::base::graph::GraphGnn;
use crate::config::Config;
use crate::ingest;
use crate::llm;
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
		let kerns = g.all();
		let mut total_entities = 0usize;
		let mut total_reasons = 0usize;
		let mut unnamed = 0usize;
		for k in &kerns {
			total_entities += k.entities.len();
			total_reasons += k.reasons.len();
			if k.is_unnamed() {
				unnamed += 1;
			}
		}
		let anchors: Vec<String> = crate::base::accept::root_anchor_ids(&g)
			.iter()
			.filter_map(|cid| g.loaded(cid))
			.map(|c| c.anchor_text.clone())
			.collect();
		let descriptors = g.root.descriptors.len();
		serde_json::json!({
			"anchors": anchors,
			"kerns": kerns.len(),
			"entities": total_entities,
			"reasons": total_reasons,
			"unnamed": unnamed,
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

pub(crate) fn parse_rfc3339(s: &str) -> Result<std::time::SystemTime, ()> {
	let s = s.trim();
	// All fixed-offset slices below read bytes 0..19. Validate length AFTER
	// trimming and require those bytes to be ASCII so the slicing can never
	// panic on a short-after-trim or multi-byte UTF-8 input (reachable from
	// untrusted MCP `since`/`before`/`valid_at` args).
	if s.len() < 19 || !s.as_bytes()[..19].is_ascii() {
		return Err(());
	}
	let year: i32 = s[0..4].parse().map_err(|_| ())?;
	let month: u32 = s[5..7].parse().map_err(|_| ())?;
	let day: u32 = s[8..10].parse().map_err(|_| ())?;
	let hour: u32 = s[11..13].parse().map_err(|_| ())?;
	let min: u32 = s[14..16].parse().map_err(|_| ())?;
	let sec: u32 = s[17..19].parse().map_err(|_| ())?;

	fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
		let y = if m <= 2 { y - 1 } else { y } as i64;
		let m = m as i64;
		let d = d as i64;
		let era = if y >= 0 { y } else { y - 399 } / 400;
		let yoe = y - era * 400;
		let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
		let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
		era * 146097 + doe - 719468
	}

	let days = days_from_civil(year, month, day);
	let secs = days * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64;
	if secs < 0 {
		return Err(());
	}
	Ok(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

#[cfg(test)]
mod parse_rfc3339_tests {
	use super::parse_rfc3339;

	#[test]
	fn valid_timestamps_parse() {
		assert!(parse_rfc3339("2026-06-05T09:00:00Z").is_ok());
		// 19 chars, no timezone suffix.
		assert!(parse_rfc3339("2026-06-05T09:00:00").is_ok());
		// Surrounding whitespace is trimmed.
		assert!(parse_rfc3339("  2026-06-05T09:00:00Z  ").is_ok());
	}

	#[test]
	fn short_after_trim_is_err_not_panic() {
		// >=20 bytes untrimmed, but trims to far fewer than 19 chars.
		assert_eq!(parse_rfc3339("   2026   "), Err(()));
		assert_eq!(parse_rfc3339("                    "), Err(())); // 20 spaces
		assert_eq!(parse_rfc3339(""), Err(()));
	}

	#[test]
	fn multibyte_in_slice_region_is_err_not_panic() {
		// 'é' (2 bytes) inside the first 19 bytes would put a str slice on a
		// non-char-boundary; must return Err, not panic.
		assert_eq!(parse_rfc3339("20é6-06-05T09:00:00Z"), Err(()));
		// Multibyte right at a split point.
		assert_eq!(parse_rfc3339("2026-06-05T09:00:0😀"), Err(()));
	}

	#[test]
	fn malformed_digits_are_err() {
		assert_eq!(parse_rfc3339("YYYY-06-05T09:00:00Z"), Err(()));
	}
}
