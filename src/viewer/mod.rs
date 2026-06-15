//! Live graph data API + zero-config local aggregator.
//!
//! Each kern daemon is per-cwd. To let one Vite app show *every* running kern
//! on the machine with no configuration, the viewer has two layers:
//!
//! 1. **Local server** — every daemon binds an ephemeral loopback port and
//!    serves its own graph at `GET /graph`. It writes that address into a
//!    shared registry directory (`<temp>/kern-viewers/<pid>.json`) and
//!    heartbeats it. A browser can't read UDP broadcasts, so the registry is a
//!    file the aggregator (a process, not the browser) reads.
//! 2. **Aggregator** — every daemon races to bind the well-known address
//!    `cfg.serve.viewer` (default 127.0.0.1:7700). Exactly one wins and becomes
//!    the hub; the rest retry periodically so the hub fails over if it dies.
//!    The hub serves `GET /graph` by fanning out to every live peer in the
//!    registry, namespacing their ids, and merging into one `{nodes,links,kerns}`.
//!
//! The browser always fetches `127.0.0.1:7700/graph` and gets the union — zero
//! config whether one daemon runs or ten.
//!
//! ## Module layout
//!
//! - [`registry`] — peer discovery: the heartbeat file + the live-peer sweep.
//! - [`local`] — the per-daemon server: this daemon's own graph, edits, tools.
//! - [`hub`] — the aggregator: cross-daemon fan-out, merge, and the oracle.

mod hub;
mod local;
mod registry;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::routing::{get, post};
use axum::Router;

use crate::base::graph::GraphGnn;
use crate::config::RetrievalConfig;

pub(crate) type Graph = Arc<RwLock<GraphGnn>>;

/// Per-peer fan-out timeout: a wedged daemon must not stall the whole view.
pub(super) const FANOUT_TIMEOUT: Duration = Duration::from_secs(3);
/// How often a non-hub daemon retries binding the aggregator address.
const FAILOVER_RETRY: Duration = Duration::from_secs(4);
/// Upper bound on a single search request's `k`, so an over-large request can't
/// drive the HNSW `ef` budget into a multi-second scan while holding the read lock.
pub(super) const MAX_SEARCH_K: usize = 200;

/// Run the viewer: start this daemon's local graph server, register it, and
/// contend for the aggregator role. Never returns under normal operation.
pub async fn run(
	graph: Graph,
	llm: crate::llm::Client,
	retrieval: RetrievalConfig,
	queue: std::sync::Arc<crate::tick::queue::Queue>,
	mcp: Arc<crate::mcp::Server>,
	agg_addr: &str,
) -> std::io::Result<()> {
	// 1. Local graph server on an ephemeral loopback port (this daemon's own data).
	let local = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
	let local_addr = local.local_addr()?.to_string();
	let local_state = local::LocalState {
		graph: graph.clone(),
		retrieval: retrieval.clone(),
		queue: queue.clone(),
		mcp,
	};
	let local_app = Router::new()
		.route("/graph", get(local::graph_json))
		.route("/ask_retrieve", post(local::ask_retrieve))
		.route("/edit", post(local::edit))
		.route("/tool", post(local::local_tool))
		.with_state(local_state);
	tokio::spawn(async move {
		if let Err(e) = axum::serve(local, local_app).await {
			tracing::warn!(target: "kern.viewer", error = %e, "local graph server exited");
		}
	});
	tracing::info!(target: "kern.viewer", addr = %local_addr, "local graph server listening");

	// 2. Register self + heartbeat so the hub can discover this daemon.
	registry::spawn_registry(local_addr.clone());

	// 3. Contend for the aggregator address; retry so the hub can fail over.
	let client = reqwest::Client::builder()
		.timeout(FANOUT_TIMEOUT)
		.build()
		.unwrap_or_default();
	let agg_addr = agg_addr.to_string();
	loop {
		match tokio::net::TcpListener::bind(&agg_addr).await {
			Ok(listener) => {
				tracing::info!(target: "kern.viewer", addr = %agg_addr, "aggregator hub listening");
				let hub = hub::HubState {
					client: client.clone(),
					llm: llm.clone(),
				};
				let app = Router::new()
					.route("/", get(hub::index))
					.route("/graph", get(hub::aggregate))
					.route("/ask", post(hub::ask))
					.route("/edit", post(hub::hub_edit))
					.route("/tool", post(hub::hub_tool))
					.with_state(hub);
				if let Err(e) = axum::serve(listener, app).await {
					tracing::warn!(target: "kern.viewer", error = %e, "aggregator hub exited; will retry");
				}
			}
			// Another daemon holds the hub. Wait, then retry to take over if it dies.
			Err(_) => tokio::time::sleep(FAILOVER_RETRY).await,
		}
	}
}
