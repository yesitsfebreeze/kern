use std::sync::Arc;

use serde_json::Value;
use trnsprt::kern_rpc::{
	CallToolReq, CallToolRes, HealthRes, KernRpc, ListToolsReq, ListToolsRes, ShutdownRes,
};
use trnsprt::typed::{Channel, JsonEnvelopeCodec, LocalListener};
use trnsprt::McpServer;

#[derive(Clone)]
pub struct KernRpcHandler {
	pub kern: Arc<crate::mcp::Server>,
	// Fires the daemon's graceful-exit path (save then exit) — the hub's unload.
	pub shutdown: Arc<tokio::sync::Notify>,
}

impl KernRpcHandler {
	pub fn new(kern: Arc<crate::mcp::Server>, shutdown: Arc<tokio::sync::Notify>) -> Self {
		Self { kern, shutdown }
	}
}

fn unwrap_tool_json(envelope: &Value) -> Result<Value, String> {
	if envelope.get("isError").and_then(|v| v.as_bool()) == Some(true) {
		let msg = envelope
			.pointer("/content/0/text")
			.and_then(|v| v.as_str())
			.unwrap_or("kern tool error")
			.to_string();
		return Err(msg);
	}
	let text = envelope
		.pointer("/content/0/text")
		.and_then(|v| v.as_str())
		.unwrap_or("");
	if text.is_empty() {
		return Ok(Value::Null);
	}
	serde_json::from_str(text).map_err(|e| format!("decode tool result: {e}"))
}

impl KernRpc for KernRpcHandler {
	fn shutdown(&self) -> impl ::core::future::Future<Output = ShutdownRes> + Send {
		let notify = self.shutdown.clone();
		async move {
			notify.notify_one();
			ShutdownRes { ok: true }
		}
	}

	fn health(&self) -> impl ::core::future::Future<Output = HealthRes> + Send {
		let kern = self.kern.clone();
		async move {
			let env = kern.tool_health();
			let payload = match unwrap_tool_json(&env) {
				Ok(v) => v,
				Err(_) => return HealthRes::default(),
			};
			let kerns = payload.get("kerns").and_then(|v| v.as_u64()).unwrap_or(0);
			let entities = payload
				.get("entities")
				.and_then(|v| v.as_u64())
				.unwrap_or(0);
			let data_dir = payload
				.get("data_dir")
				.and_then(|v| v.as_str())
				.unwrap_or("")
				.to_string();
			let u64_at = |k: &str| payload.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
			HealthRes {
				ok: true,
				data_dir,
				kerns,
				entities,
				idle_ms: kern.idle_ms(),
				queue_depth: u64_at("queue_depth"),
				tasks_done: u64_at("tasks_done"),
				task_avg_ms: u64_at("task_avg_ms"),
			}
		}
	}

	fn call_tool(
		&self,
		req: CallToolReq,
	) -> impl ::core::future::Future<Output = CallToolRes> + Send {
		let kern = self.kern.clone();
		async move {
			// Forward to the single MCP `call_tool` dispatcher so the proxy relays any
			// `tools/call` over kern.sock without enumerating every tool.
			let envelope = match McpServer::call_tool(&*kern, &req.name, &req.args) {
				Ok(tr) => serde_json::json!({
						"content": tr.content,
						"isError": tr.is_error,
				}),
				Err(e) => serde_json::json!({
						"content": [{
								"type": "text",
								"text": format!("kern_rpc::call_tool: {e}"),
						}],
						"isError": true,
				}),
			};
			CallToolRes { envelope }
		}
	}

	fn list_tools(
		&self,
		_req: ListToolsReq,
	) -> impl ::core::future::Future<Output = ListToolsRes> + Send {
		let kern = self.kern.clone();
		async move {
			// Serialise the live `tools/list` to raw schemas so the proxy advertises
			// exactly what we expose.
			let tools = McpServer::tools_list(&*kern)
				.iter()
				.filter_map(|s| serde_json::to_value(s).ok())
				.collect();
			ListToolsRes { tools }
		}
	}
}

pub async fn serve_kern_rpc_loop(mut listener: LocalListener, handler: KernRpcHandler) {
	loop {
		let adapter = match listener.accept().await {
			Ok(a) => a,
			Err(e) => {
				tracing::warn!(target: "kern.kern_rpc", error = %e, "accept");
				continue;
			}
		};
		let handler = handler.clone();
		tokio::spawn(async move {
			let channel = Channel::new(adapter, JsonEnvelopeCodec::new());
			if let Err(e) = trnsprt::kern_rpc::serve_kern_rpc(channel, handler).await {
				tracing::warn!(target: "kern.kern_rpc", error = %e, "serve loop");
			}
		});
	}
}
