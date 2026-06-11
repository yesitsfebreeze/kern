//! MCP Streamable HTTP transport (2025 spec).
//!
//! `POST /mcp` — client request → JSON response.
//! `GET  /mcp` — SSE stream for server-initiated notifications.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use crate::server::{dispatch, error_response};
use crate::McpServer;

/// Serve MCP Streamable HTTP on `addr` (e.g. `"127.0.0.1:3001"`).
pub async fn serve_http<S>(server: Arc<S>, addr: &str) -> std::io::Result<()>
where
	S: McpServer + Sync + 'static,
{
	let app = Router::new()
		.route("/mcp", post(handle_post::<S>))
		.route("/mcp", get(handle_get))
		.with_state(server);

	let listener = tokio::net::TcpListener::bind(addr).await?;
	axum::serve(listener, app).await?;
	Ok(())
}

async fn handle_post<S: McpServer + Sync + 'static>(
	State(server): State<Arc<S>>,
	headers: HeaderMap,
	body: String,
) -> impl IntoResponse {
	// Reject an explicit non-JSON Content-Type up front. A missing header is
	// allowed (many MCP clients omit it) — the JSON parse below still rejects a
	// non-JSON body with -32700, so this only hardens the mislabelled-payload case.
	if let Some(ct) = headers.get(axum::http::header::CONTENT_TYPE) {
		let is_json = ct
			.to_str()
			.map(|s| s.trim_start().starts_with("application/json"))
			.unwrap_or(false);
		if !is_json {
			let resp = error_response(
				serde_json::Value::Null,
				-32700,
				"content-type must be application/json",
			);
			return (StatusCode::OK, axum::Json(resp));
		}
	}

	let frame: serde_json::Value = match serde_json::from_str(&body) {
		Ok(v) => v,
		Err(e) => {
			let resp = error_response(serde_json::Value::Null, -32700, &format!("parse error: {e}"));
			return (StatusCode::OK, axum::Json(resp));
		}
	};

	match dispatch(server.as_ref(), &frame) {
		Some(resp) => (StatusCode::OK, axum::Json(resp)),
		None => (StatusCode::ACCEPTED, axum::Json(serde_json::Value::Null)),
	}
}

async fn handle_get() -> impl IntoResponse {
	let stream = async_stream::stream! {
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(25)).await;
			yield Ok::<Event, std::convert::Infallible>(Event::default().comment("keepalive"));
		}
	};
	Sse::new(stream)
}

#[cfg(test)]
mod tests {
	use super::*;
	use axum::http::header::CONTENT_TYPE;
	use serde_json::{json, Value};

	// Local mock instead of test-utils::AdderServer: trnsprt has a dev-dependency
	// CYCLE (trnsprt -> test-utils -> trnsprt), so within trnsprt's own unit-test
	// build, AdderServer impls a *different* McpServer instance than this harness
	// sees. A mock over this crate's own trait sidesteps that. (The cross-crate
	// AdderServer is still used by shared/trnsprt/tests/integration.rs, a separate
	// crate where both see the same trnsprt.)
	struct MockServer;
	impl McpServer for MockServer {
		fn tools_list(&self) -> Vec<crate::ToolSchema> {
			vec![crate::ToolSchema {
				name: "add".into(),
				description: Some("a+b".into()),
				input_schema: None,
			}]
		}
		fn call_tool(&self, name: &str, _args: &Value) -> Result<crate::ToolResult, crate::McpError> {
			if name == "add" {
				Ok(crate::ToolResult {
					content: vec![json!({ "type": "text", "text": "ok" })],
					is_error: false,
					structured_content: None,
				})
			} else {
				Err(crate::McpError::Rpc { code: -32601, message: format!("unknown tool: {name}") })
			}
		}
	}

	async fn body_json(resp: axum::response::Response) -> Value {
		let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	#[tokio::test]
	async fn post_tools_list_returns_a_result_listing_the_tool() {
		let server = Arc::new(MockServer);
		let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list" }).to_string();
		let resp = handle_post(State(server), HeaderMap::new(), body).await.into_response();
		assert_eq!(resp.status(), StatusCode::OK);
		let v = body_json(resp).await;
		assert_eq!(v["id"], 7, "id is echoed back");
		assert!(v.get("error").map(Value::is_null).unwrap_or(true), "no error: {v}");
		assert!(v["result"].is_object(), "a result object is present: {v}");
		assert!(serde_json::to_string(&v).unwrap().contains("add"), "the add tool is listed");
	}

	#[tokio::test]
	async fn post_non_json_body_returns_parse_error() {
		let server = Arc::new(MockServer);
		let resp = handle_post(State(server), HeaderMap::new(), "not json".into())
			.await
			.into_response();
		assert_eq!(resp.status(), StatusCode::OK);
		let v = body_json(resp).await;
		assert_eq!(v["error"]["code"], -32700, "malformed JSON is a parse error");
	}

	#[tokio::test]
	async fn post_with_non_json_content_type_is_rejected() {
		let server = Arc::new(MockServer);
		let mut headers = HeaderMap::new();
		headers.insert(CONTENT_TYPE, "text/plain".parse().unwrap());
		// Body is valid JSON, but the mislabelled Content-Type is rejected up front.
		let resp = handle_post(State(server), headers, "{}".into()).await.into_response();
		let v = body_json(resp).await;
		assert_eq!(v["error"]["code"], -32700, "wrong content-type -> -32700");
	}
}
