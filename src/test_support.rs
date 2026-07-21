use crate::base::types::{Entity, Reason};
use tokio::task::JoinHandle;

pub(crate) fn entity(id: &str) -> Entity {
	Entity {
		id: id.into(),
		..Default::default()
	}
}

pub(crate) fn entity_vec(id: &str, vector: Vec<f32>) -> Entity {
	Entity {
		id: id.into(),
		vector: vector.into(),
		..Default::default()
	}
}

pub(crate) fn edge(from: &str, to: &str) -> Reason {
	Reason {
		id: format!("{from}->{to}"),
		from: from.into(),
		to: to.into(),
		..Default::default()
	}
}

// A dead port: nothing in the default rig should reach an embedder.
pub(crate) fn mcp_server() -> crate::mcp::Server {
	mcp_server_with_embed_url("http://127.0.0.1:1")
}

// Same server against a live stub embedder, for tests that have to follow an
// ingest all the way into the graph rather than stop at the tool boundary.
pub(crate) fn mcp_server_with_embed_url(url: &str) -> crate::mcp::Server {
	use parking_lot::RwLock;
	use std::sync::Arc;
	let graph = Arc::new(RwLock::new(crate::base::graph::GraphGnn::new()));
	let embedder = crate::llm::Client::new_embed_only(url, "test", "");
	let worker = Arc::new(crate::ingest::Worker::new(
		graph.clone(),
		embedder,
		None,
		None,
		None,
	));
	crate::mcp::Server {
		graph,
		worker,
		llm: None,
		save_fn: Arc::new(|| {}),
		task_q: None,
		cfg: Arc::new(crate::config::Config::default()),
		broadcast_pulse: None,
		last_activity: Arc::new(std::sync::atomic::AtomicU64::new(
			crate::base::util::now_ms(),
		)),
	}
}

#[cfg(unix)]
pub(crate) fn scratch_endpoint(tag: &str) -> trnsprt::typed::Endpoint {
	let dir = std::env::temp_dir().join(format!(
		"kern-route-{}-{}-{tag}",
		std::process::id(),
		crate::base::util::now_ms()
	));
	std::fs::create_dir_all(&dir).expect("scratch dir");
	trnsprt::typed::Endpoint::Unix(dir.join("kern.sock"))
}

#[cfg(unix)]
pub(crate) async fn serving(srv: crate::mcp::Server, endpoint: &trnsprt::typed::Endpoint) {
	use std::sync::Arc;
	use trnsprt::typed::{bind_kern_listener, BindOutcome};

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

pub(crate) fn tool_text(v: &serde_json::Value) -> String {
	v["content"][0]["text"].as_str().unwrap_or("").to_string()
}

// An embed endpoint that never answers. Pins the ingest worker on one job so a
// test can fill the queue behind it.
pub(crate) fn hanging_embed_app() -> axum::Router {
	axum::Router::new().route(
		"/api/embed",
		axum::routing::post(|_b: axum::Json<serde_json::Value>| async move {
			std::future::pending::<axum::Json<serde_json::Value>>().await
		}),
	)
}

pub(crate) async fn spawn_http(app: axum::Router) -> (String, JoinHandle<()>) {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
	(format!("http://{addr}"), handle)
}

// A second writer committing straight through the shared store — how a daemon
// advances the epoch underneath a one-shot CLI command mid-flight.
pub(crate) fn commit_extra_kern_via_store(
	g: &std::sync::Arc<parking_lot::RwLock<crate::base::graph::GraphGnn>>,
	kern: crate::base::types::Kern,
) {
	let gg = g.read();
	let store = gg.store().expect("graph has a bound store");
	let mut kerns = std::collections::HashMap::new();
	for k in gg.all() {
		kerns.insert(k.id.clone(), k.clone());
	}
	kerns.insert(gg.root.id.clone(), gg.root.clone());
	kerns.insert(kern.id.clone(), kern);
	store
		.save_all_kerns(&kerns, &gg.network_id, gg.quant_mode)
		.expect("external commit through the shared store");
}
