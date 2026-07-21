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
			let str_at = |k: &str| {
				payload
					.get(k)
					.and_then(|v| v.as_str())
					.unwrap_or("")
					.to_string()
			};
			HealthRes {
				ok: true,
				data_dir,
				kerns,
				entities,
				idle_ms: kern.idle_ms(),
				queue_depth: u64_at("queue_depth"),
				tasks_done: u64_at("tasks_done"),
				task_avg_ms: u64_at("task_avg_ms"),
				task_panics: u64_at("task_panics"),
				last_task_panic: str_at("last_task_panic"),
				task_failures: u64_at("task_failures"),
				last_task_failure: str_at("last_task_failure"),
				cold_evicted: u64_at("cold_evicted"),
				query_dim_rejected: u64_at("query_dim_rejected"),
				below_floor_deliveries: u64_at("below_floor_deliveries"),
				clock_skew_skips: u64_at("clock_skew_skips"),
				ingest_dropped_chunks: u64_at("ingest_dropped_chunks"),
				remote_cap_dropped: u64_at("remote_cap_dropped"),
				unspilled_drops: u64_at("unspilled_drops"),
				ingest_queue_refused: u64_at("ingest_queue_refused"),
				embed_model: str_at("embed_model"),
				embed_dim: u64_at("embed_dim"),
				embed_mismatch: payload
					.get("embed_mismatch")
					.and_then(|v| v.as_bool())
					.unwrap_or(false),
				build_id: crate::base::identity::build_id(),
				config_id: crate::base::identity::config_id(&kern.cfg),
				uptime_ms: crate::base::identity::uptime_ms(),
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tick::queue::{task, Queue, TaskKind};

	#[tokio::test]
	async fn health_carries_every_degradation_signal_to_the_rpc_surface() {
		let mut srv = crate::test_support::mcp_server();
		let q = Arc::new(Queue::new(8));
		q.record_task_panic(&task(TaskKind::Cluster, "k1"), "boom");
		q.record_task_failure(&task(TaskKind::GnnPropagate, "k2"), "train epoch 0 forward");
		srv.task_q = Some(q);

		let handler = KernRpcHandler::new(Arc::new(srv), Arc::new(tokio::sync::Notify::new()));
		let res = handler.health().await;

		assert!(res.ok);
		assert_eq!(res.task_panics, 1);
		assert_eq!(res.last_task_panic, "Cluster[k1]: boom");
		assert_eq!(res.task_failures, 1);
		assert_eq!(
			res.last_task_failure,
			"GnnPropagate[k2]: train epoch 0 forward"
		);
		assert_eq!(res.cold_evicted, 0);
		assert!(!res.embed_mismatch);
	}

	// Not "the key is present" — a real refusal, walked from the worker's counter
	// through the health stats and the MCP payload to the RPC DTO an operator polls.
	#[tokio::test]
	async fn a_refused_ingest_reaches_the_rpc_health_surface() {
		let (url, _server) =
			crate::test_support::spawn_http(crate::test_support::hanging_embed_app()).await;
		let srv = crate::test_support::mcp_server_with_embed_url(&url);

		let mut offered = 0;
		while srv
			.worker
			.enqueue(
				format!("filler {offered}"),
				crate::base::types::Source::Inline {
					hash: String::new(),
					section: String::new(),
				},
				crate::base::types::EntityKind::Claim,
				String::new(),
				1.0,
				crate::ingest::Config::default(),
			)
			.is_some()
		{
			offered += 1;
			tokio::task::yield_now().await;
			assert!(offered < 10_000, "the queue never filled");
		}

		let handler = KernRpcHandler::new(Arc::new(srv), Arc::new(tokio::sync::Notify::new()));
		assert!(
			handler.health().await.ingest_queue_refused >= 1,
			"a refused ingest that no health surface reports is a lost write nobody can see"
		);
	}
}
