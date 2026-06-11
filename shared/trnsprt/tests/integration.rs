use serde_json::json;
use test_utils::mcp_pipe::{new_pipe, reply_error, reply_result, AdderServer};
use trnsprt::{
	Client, InProcTransport, LiveServer, McpError, Registry, ServerId, ToolSchema,
	PROTOCOL_VERSION,
};

/// In-process round-trip latency budget. The transport is a direct function
/// call (no IO/syscalls), so per-call overhead should be well under a
/// millisecond; 5 ms is a deliberately loose regression ceiling that still
/// catches an accidental blocking call or per-call allocation on the hot path.
const PER_CALL_CEILING: std::time::Duration = std::time::Duration::from_millis(5);
/// Calls averaged in the latency probe to smooth scheduler/timer noise.
const LATENCY_ITERS: u32 = 100;

#[test]
fn handshake_sends_initialize_and_initialized_notification() {
	let (transport, wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	wire.push_reply(&reply_result(1, json!({ "protocolVersion": PROTOCOL_VERSION })));
	let result = client.initialize("kern", "test").expect("initialize");
	assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
	let frames = wire.drain_frames();
	assert_eq!(frames.len(), 2, "init request + initialized notification");
	assert_eq!(frames[0]["method"], "initialize");
	assert_eq!(frames[0]["id"], 1);
	assert_eq!(frames[1]["method"], "notifications/initialized");
	assert!(frames[1].get("id").is_none(), "notification has no id");
}

#[test]
fn tools_list_returns_parsed_schemas() {
	let (transport, wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	wire.push_reply(&reply_result(
		1,
		json!({
			"tools": [
				{
					"name": "echo",
					"description": "echo args",
					"inputSchema": { "type": "object" },
				},
				{ "name": "ping" },
			]
		}),
	));
	let tools = client.list_tools().expect("list_tools");
	assert_eq!(tools.len(), 2);
	assert_eq!(tools[0].name, "echo");
	assert_eq!(tools[0].description.as_deref(), Some("echo args"));
	assert_eq!(tools[1].name, "ping");
	assert!(tools[1].description.is_none());
}

#[test]
fn tools_call_returns_typed_result() {
	let (transport, wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	wire.push_reply(&reply_result(
		1,
		json!({
			"content": [{ "type": "text", "text": "hi" }],
			"isError": false,
		}),
	));
	let out = client
		.call_tool("echo", &json!({ "msg": "hi" }))
		.expect("call_tool");
	assert!(!out.is_error);
	assert_eq!(out.content.len(), 1);
	assert_eq!(out.content[0]["text"], "hi");
	let frames = wire.drain_frames();
	assert_eq!(frames[0]["method"], "tools/call");
	assert_eq!(frames[0]["params"]["name"], "echo");
	assert_eq!(frames[0]["params"]["arguments"]["msg"], "hi");
}

#[test]
fn rpc_error_is_surfaced_as_result() {
	let (transport, wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	wire.push_reply(&reply_error(1, -32601, "method not found"));
	let err = client.list_tools().expect_err("must error");
	match err {
		McpError::Rpc { code, message } => {
			assert_eq!(code, -32601);
			assert_eq!(message, "method not found");
		}
		other => panic!("expected Rpc, got {other:?}"),
	}
}

#[test]
fn eof_before_response_yields_not_running() {
	let (transport, _wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	let err = client.list_tools().expect_err("must error");
	assert!(matches!(err, McpError::NotRunning), "got {err:?}");
}

#[test]
fn drop_invokes_transport_kill() {
	let (transport, wire) = new_pipe();
	{
		let _client = Client::new(Box::new(transport));
	}
	assert!(wire.killed(), "kill() must be called on drop");
}

#[test]
fn out_of_order_notification_is_skipped() {
	let (transport, wire) = new_pipe();
	let mut client = Client::new(Box::new(transport));
	wire.push_reply(&json!({ "jsonrpc": "2.0", "method": "notifications/log", "params": {} }));
	wire.push_reply(&reply_result(1, json!({ "tools": [] })));
	let tools = client.list_tools().expect("list_tools");
	assert!(tools.is_empty());
}

#[test]
fn registry_reports_unknown_server() {
	let mut reg = Registry::new();
	let err = reg
		.call_tool(&ServerId::new("nope"), "x", &json!({}))
		.expect_err("must error");
	assert!(matches!(err, McpError::UnknownServer(_)));
}

#[test]
fn inproc_transport_roundtrips_handshake_and_tools_list() {
	let mut client = Client::new(Box::new(InProcTransport::new(Box::new(AdderServer))));
	let result = client.initialize("kern", "test").expect("initialize");
	assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
	let tools = client.list_tools().expect("list_tools");
	assert_eq!(tools.len(), 1);
	assert_eq!(tools[0].name, "add");
}

#[test]
fn inproc_transport_dispatches_tools_call() {
	let mut client = Client::new(Box::new(InProcTransport::new(Box::new(AdderServer))));
	client.initialize("kern", "test").expect("initialize");
	let out = client
		.call_tool("add", &json!({ "a": 2, "b": 40 }))
		.expect("call_tool");
	assert!(!out.is_error);
	assert_eq!(out.content[0]["text"], "42");
}

#[test]
fn inproc_transport_surfaces_tool_error_as_rpc() {
	let mut client = Client::new(Box::new(InProcTransport::new(Box::new(AdderServer))));
	client.initialize("kern", "test").expect("initialize");
	let err = client
		.call_tool("nope", &json!({}))
		.expect_err("unknown tool");
	assert!(matches!(err, McpError::Rpc { code: -32601, .. }));
}

#[test]
fn registry_register_inproc_routes_call_tool() {
	let mut reg = Registry::new();
	let id = ServerId::new("math");
	reg
		.register_inproc(id.clone(), Box::new(AdderServer))
		.expect("register");
	let listed = reg.list_tools(&id).expect("list_tools");
	assert_eq!(listed.len(), 1);
	let out = reg
		.call_tool(&id, "add", &json!({ "a": 7, "b": 8 }))
		.expect("call_tool");
	assert_eq!(out.content[0]["text"], "15");
}

#[test]
fn inproc_transport_call_latency_under_ceiling() {
	let mut client = Client::new(Box::new(InProcTransport::new(Box::new(AdderServer))));
	client.initialize("kern", "test").expect("initialize");
	let _ = client.call_tool("add", &json!({ "a": 0, "b": 0 })).unwrap();
	let start = std::time::Instant::now();
	for _ in 0..LATENCY_ITERS {
		let _ = client.call_tool("add", &json!({ "a": 1, "b": 2 })).unwrap();
	}
	let per_call = start.elapsed() / LATENCY_ITERS;
	assert!(
		per_call < PER_CALL_CEILING,
		"in-proc per-call latency regressed: {per_call:?} (ceiling {PER_CALL_CEILING:?})"
	);
}

#[test]
fn inproc_call_tool_missing_argument_is_invalid_params() {
	// AdderServer requires both operands; omitting one surfaces a -32602
	// (Invalid params) Rpc error through the in-process transport + client.
	let mut client = Client::new(Box::new(InProcTransport::new(Box::new(AdderServer))));
	client.initialize("kern", "test").expect("initialize");
	let err = client
		.call_tool("add", &json!({ "a": 1 })) // missing "b"
		.expect_err("missing argument must error");
	assert!(matches!(err, McpError::Rpc { code: -32602, .. }), "got {err:?}");
}

#[test]
fn registry_register_inproc_rejects_duplicate_server_id() {
	// Public-API coverage of the duplicate-id guard (registry.rs unit tests
	// cover the same path with a local MockServer; this exercises it through
	// the integration crate's public Registry surface with AdderServer).
	let mut reg = Registry::new();
	let id = ServerId::new("dup");
	reg.register_inproc(id.clone(), Box::new(AdderServer)).expect("first registration");
	let again = reg.register_inproc(id.clone(), Box::new(AdderServer));
	assert!(matches!(again, Err(McpError::DuplicateServer(_))), "second register must error");
}

#[test]
fn registry_stores_and_serves_tools() {
	let (transport, wire) = new_pipe();
	let client = Client::new(Box::new(transport));
	let tools = vec![ToolSchema {
		name: "ping".into(),
		description: None,
		input_schema: None,
	}];
	let server = LiveServer::new(client, tools);
	let mut reg = Registry::new();
	let id = ServerId::new("srv1");
	reg.insert(id.clone(), server);
	let listed = reg.list_tools(&id).expect("list_tools");
	assert_eq!(listed.len(), 1);
	assert_eq!(listed[0].name, "ping");

	wire.push_reply(&reply_result(1, json!({ "content": [], "isError": false })),);
	let out = reg.call_tool(&id, "ping", &json!({})).expect("call_tool");
	assert!(!out.is_error);
}
