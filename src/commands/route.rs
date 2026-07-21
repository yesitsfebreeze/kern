use trnsprt::kern_rpc::{CallToolReq, KernRpcClient};
use trnsprt::typed::{Endpoint, JsonEnvelopeCodec};

pub(crate) enum Routed {
	Done(serde_json::Value),
	Refused(String),
	NoDaemon,
}

pub(crate) async fn route(name: &str, args: serde_json::Value) -> Routed {
	route_to(&Endpoint::kern(), name, args).await
}

// One attempt and no spawn: a one-shot write must never conjure the daemon it
// was looking for, and an absent socket is the ordinary no-daemon case rather
// than a failure to report.
pub(crate) async fn route_to(endpoint: &Endpoint, name: &str, args: serde_json::Value) -> Routed {
	let connected = KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(
		endpoint,
		1,
		std::time::Duration::ZERO,
	)
	.await;
	let Ok(client) = connected else {
		return Routed::NoDaemon;
	};
	let req = CallToolReq {
		name: name.to_string(),
		args,
	};
	match client.call_tool(req).await {
		Ok(res) => unwrap_envelope(&res.envelope),
		// A daemon that answered the connect owns the graph even if the call went
		// wrong. Writing locally behind its back is exactly the split this closes,
		// so a failed call is reported, never retried against the store.
		Err(e) => Routed::Refused(format!("daemon rpc: {e}")),
	}
}

fn unwrap_envelope(envelope: &serde_json::Value) -> Routed {
	let text = envelope
		.pointer("/content/0/text")
		.and_then(|v| v.as_str())
		.unwrap_or("");
	if envelope.get("isError").and_then(|v| v.as_bool()) == Some(true) {
		let msg = if text.is_empty() {
			"kern tool error".to_string()
		} else {
			text.to_string()
		};
		return Routed::Refused(msg);
	}
	match serde_json::from_str(text) {
		Ok(v) => Routed::Done(v),
		Err(e) => Routed::Refused(format!("decode daemon result: {e}")),
	}
}

pub(crate) fn u64_field(v: &serde_json::Value, key: &str) -> u64 {
	v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

#[cfg(all(test, unix))]
mod tests {
	use super::*;
	use std::sync::Arc;
	use trnsprt::typed::{bind_kern_listener, BindOutcome};

	fn scratch_endpoint(tag: &str) -> Endpoint {
		let dir = std::env::temp_dir().join(format!(
			"kern-route-{}-{}-{tag}",
			std::process::id(),
			crate::base::util::now_ms()
		));
		std::fs::create_dir_all(&dir).expect("scratch dir");
		Endpoint::Unix(dir.join("kern.sock"))
	}

	async fn serving(srv: crate::mcp::Server, endpoint: &Endpoint) {
		let BindOutcome::Bound(listener) = bind_kern_listener(endpoint).await.expect("bind") else {
			panic!("scratch endpoint already bound");
		};
		let handler = crate::rpc::kern_rpc_server::KernRpcHandler::new(
			Arc::new(srv),
			Arc::new(tokio::sync::Notify::new()),
		);
		tokio::spawn(crate::rpc::kern_rpc_server::serve_kern_rpc_loop(
			listener, handler,
		));
	}

	fn kern_with_edge() -> crate::mcp::Server {
		use crate::base::reason::add_reason;
		use crate::base::types::Kern;
		let srv = crate::test_support::mcp_server();
		let mut k = Kern::new("kx", "");
		k.entities
			.insert("a".into(), crate::test_support::entity("a"));
		k.entities
			.insert("b".into(), crate::test_support::entity("b"));
		let mut healthy = crate::test_support::edge("a", "b");
		healthy.score = 1.0;
		add_reason(&mut k, healthy);
		add_reason(&mut k, crate::test_support::edge("a", "c"));
		srv.graph.write().kerns.insert("kx".into(), k);
		srv
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn a_missing_socket_is_no_daemon_not_an_error() {
		let ep = scratch_endpoint("absent");
		let out = route_to(&ep, "forget", serde_json::json!({"id": "a"})).await;
		assert!(
			matches!(out, Routed::NoDaemon),
			"nothing serving -> the caller owns the write"
		);
	}

	// The whole point of item 9's serving half: the mutation must land in the
	// daemon's live graph, not in a second copy the CLI opened behind it.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_routed_forget_mutates_the_serving_daemons_graph() {
		let ep = scratch_endpoint("forget");
		let srv = kern_with_edge();
		let graph = srv.graph.clone();
		serving(srv, &ep).await;

		let out = route_to(&ep, "forget", serde_json::json!({"id": "a"})).await;
		let Routed::Done(v) = out else {
			panic!("a serving daemon must answer the forget");
		};
		assert_eq!(
			u64_field(&v, "removed_edges"),
			2,
			"both incident edges cascaded"
		);

		let g = graph.read();
		assert!(
			!g.kerns.get("kx").expect("kern").entities.contains_key("a"),
			"the daemon's own graph lost the entity"
		);
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn a_routed_degrade_reports_both_counts_from_the_daemon() {
		let ep = scratch_endpoint("degrade");
		serving(kern_with_edge(), &ep).await;

		let out = route_to(&ep, "degrade", serde_json::json!({"query_id": "a"})).await;
		let Routed::Done(v) = out else {
			panic!("a serving daemon must answer the degrade");
		};
		assert_eq!(
			u64_field(&v, "decayed_edges"),
			2,
			"both incident edges visited"
		);
		// The CLI has always printed a reap count; a routed degrade that cannot
		// read one back would silently print 0 for every reap.
		assert_eq!(
			u64_field(&v, "removed_edges"),
			1,
			"the sub-threshold edge is reported as reaped"
		);
	}

	// A tool error is the daemon's answer, not a reason to go around it.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_daemon_tool_error_is_refused_not_downgraded_to_no_daemon() {
		let ep = scratch_endpoint("refuse");
		serving(kern_with_edge(), &ep).await;

		let out = route_to(&ep, "forget", serde_json::json!({"id": "ghost"})).await;
		match out {
			Routed::Refused(msg) => assert!(msg.contains("thought not found"), "{msg}"),
			_ => panic!("an unknown id must come back as the daemon's refusal"),
		}
	}
}
