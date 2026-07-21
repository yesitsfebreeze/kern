use parking_lot::RwLock as StdRwLock;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use trnsprt::kern_rpc::{CallToolReq, KernRpcClient};
use trnsprt::typed::{AdapterError, Endpoint, JsonEnvelopeCodec};
use trnsprt::{McpError, McpServer, ToolResult, ToolSchema};

use super::load_graph;

pub(super) async fn cmd_mcp(cfg: &crate::config::Config) {
	// Hub-first: a running hub owns node lifecycle (spawn, adopt, unload) so the
	// proxy never self-spawns a daemon the hub can't see. No hub -> direct path.
	let log_dir = cfg.log_dir();
	if let Some(client) = attach_via_hub(cfg.hub.auto_start, &log_dir).await {
		let client = replace_if_stale(client, cfg, &log_dir, true).await;
		run_proxy(client).await;
		return;
	}
	match attach_with_retry(2, 150).await {
		Ok(client) => {
			let client = replace_if_stale(client, cfg, &log_dir, false).await;
			run_proxy(client).await;
		}
		Err(e_first) => {
			tracing::info!(
				target: "kern.mcp",
				error = %e_first,
				"no daemon at kern.sock — auto-spawning detached daemon"
			);
			match spawn_daemon(&log_dir) {
				Ok(()) => match attach_with_retry(6, 150).await {
					Ok(client) => {
						tracing::info!(
							target: "kern.mcp_proxy",
							"attached to auto-spawned daemon — proxy mode"
						);
						run_proxy(client).await;
					}
					Err(e_retry) => {
						tracing::warn!(
							target: "kern.mcp",
							error = %e_retry,
							"auto-spawn failed, falling back to standalone"
						);
						run_standalone(cfg).await;
					}
				},
				Err(e_spawn) => {
					tracing::warn!(
						target: "kern.mcp",
						error = %e_spawn,
						"auto-spawn failed, falling back to standalone"
					);
					run_standalone(cfg).await;
				}
			}
		}
	}
}

async fn run_proxy(client: KernRpcClient<JsonEnvelopeCodec>) {
	tracing::info!(
		target: "kern.mcp_proxy",
		"attached to running daemon — proxy mode"
	);
	let proxy = ProxyServer {
		client: Arc::new(TokioMutex::new(client)),
	};
	// serve_stdio is sync — on a blocking thread so it doesn't park a worker;
	// call_tool crosses back via block_in_place (multi-thread rt only).
	if let Err(e) = tokio::task::spawn_blocking(move || trnsprt::serve_stdio(&proxy)).await {
		tracing::warn!(target: "kern.mcp_proxy", error = %e, "stdio loop");
	}
}

// One attempt per invocation, by construction: this runs once on the way into
// the proxy and never loops. A client whose replacement is itself stale
// proxies anyway rather than restarting again.
async fn replace_if_stale(
	client: KernRpcClient<JsonEnvelopeCodec>,
	cfg: &crate::config::Config,
	log_dir: &std::path::Path,
	via_hub: bool,
) -> KernRpcClient<JsonEnvelopeCodec> {
	use super::mcp_restart::{verdict, Verdict};
	use crate::base::identity;

	let health = match client.health().await {
		Ok(h) => h,
		// A daemon that will not answer health is not one to judge stale.
		Err(_) => return client,
	};
	let (my_build, my_config) = (identity::build_id(), identity::config_id(cfg));
	let reason = match verdict(&health, &my_build, &my_config) {
		Verdict::Fresh => return client,
		Verdict::Hold(why) => {
			if health.build_id != my_build || health.config_id != my_config {
				tracing::warn!(
					target: "kern.mcp",
					reason = why,
					daemon_build = %health.build_id,
					client_build = %my_build,
					"attached to a daemon that does not match this client — not restarting"
				);
			}
			return client;
		}
		Verdict::Stale(why) => why,
	};

	if !cfg.hub.auto_restart {
		tracing::warn!(
			target: "kern.mcp",
			reason,
			"stale daemon — set [hub] auto_restart = true to replace it automatically"
		);
		return client;
	}

	tracing::info!(target: "kern.mcp", reason, "restarting stale daemon");
	// shutdown() fires the daemon's graceful path: drain, guarded flush, exit.
	// A refused or dropped call means it is already going down.
	let _ = client.shutdown().await;
	drop(client);

	// Wait for the socket to be released before anything re-binds it, or the
	// respawn loses the race to the corpse and we reattach to what we just killed.
	for _ in 0..40 {
		tokio::time::sleep(std::time::Duration::from_millis(150)).await;
		if attach_with_retry(1, 0).await.is_err() {
			break;
		}
	}

	let fresh = if via_hub {
		attach_via_hub(cfg.hub.auto_start, log_dir).await
	} else {
		match spawn_daemon(log_dir) {
			Ok(()) => attach_with_retry(40, 250).await.ok(),
			Err(e) => {
				tracing::warn!(target: "kern.mcp", error = %e, "respawn after restart failed");
				None
			}
		}
	};
	match fresh {
		Some(c) => {
			tracing::info!(target: "kern.mcp", "reattached to restarted daemon");
			c
		}
		// Fail open: no memory is recoverable, a dead proxy is not.
		None => {
			tracing::warn!(
				target: "kern.mcp",
				"could not reattach after restart — falling back to a fresh attach"
			);
			match attach_with_retry(40, 250).await {
				Ok(c) => c,
				Err(e) => {
					tracing::error!(target: "kern.mcp", error = %e, "no daemon after restart");
					std::process::exit(1);
				}
			}
		}
	}
}

async fn attach_via_hub(
	auto_start: bool,
	log_dir: &std::path::Path,
) -> Option<KernRpcClient<JsonEnvelopeCodec>> {
	use trnsprt::hub_rpc::{HubRpcClient, ResolveReq};
	let hub = match HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
		Ok(h) => h,
		Err(_) if auto_start => {
			// Same detach pattern as spawn_daemon; a lost race lands on
			// AlreadyRunning in the second hub and the retry below still connects.
			if let Err(e) = spawn_hub(log_dir) {
				tracing::warn!(target: "kern.mcp", error = %e, "hub auto-start failed — direct path");
				return None;
			}
			let mut connected = None;
			for _ in 0..6 {
				tokio::time::sleep(std::time::Duration::from_millis(150)).await;
				if let Ok(h) = HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
					connected = Some(h);
					break;
				}
			}
			match connected {
				Some(h) => {
					tracing::info!(target: "kern.mcp", "auto-started machine hub");
					h
				}
				None => {
					tracing::warn!(target: "kern.mcp", "auto-started hub never answered — direct path");
					return None;
				}
			}
		}
		Err(_) => return None,
	};
	// main.rs re-pinned cwd to the project root before dispatch.
	let root = std::env::current_dir().ok()?;
	let res = hub
		.resolve(ResolveReq {
			root: root.display().to_string(),
		})
		.await
		.ok()?;
	if !res.ok {
		tracing::warn!(target: "kern.mcp", error = %res.err, "hub resolve failed — direct path");
		return None;
	}
	let endpoint = trnsprt::typed::Endpoint::parse(&res.endpoint);
	tracing::info!(
		target: "kern.mcp",
		endpoint = %res.endpoint,
		spawned = res.spawned,
		"attached via hub"
	);
	KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint(&endpoint)
		.await
		.ok()
}

async fn attach_with_retry(
	retries: u32,
	delay_ms: u64,
) -> Result<KernRpcClient<JsonEnvelopeCodec>, AdapterError> {
	let mut last_err: Option<AdapterError> = None;
	for i in 0..retries {
		match KernRpcClient::<JsonEnvelopeCodec>::connect_local().await {
			Ok(c) => return Ok(c),
			Err(e) => {
				last_err = Some(e);
				if i + 1 < retries {
					tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
				}
			}
		}
	}
	Err(last_err.unwrap_or_else(|| AdapterError::Other("no attempts".into())))
}

fn spawn_hub(log_dir: &std::path::Path) -> std::io::Result<()> {
	spawn_detached("hub", log_dir)
}

fn spawn_daemon(log_dir: &std::path::Path) -> std::io::Result<()> {
	spawn_detached("--daemon", log_dir)
}

// Drop the child handle — detach flags + redirected stdio keep it alive past our exit.
fn spawn_detached(arg: &str, log_dir: &std::path::Path) -> std::io::Result<()> {
	use std::process::{Command, Stdio};
	let exe = std::env::current_exe()?;
	let (out, err) = crate::config::detached_log::stdio(log_dir, arg);
	let mut cmd = Command::new(exe);
	cmd.arg(arg).stdin(Stdio::null()).stdout(out).stderr(err);
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		const DETACHED_PROCESS: u32 = 0x0000_0008;
		const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
		cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
	}
	#[cfg(unix)]
	{
		use std::os::unix::process::CommandExt;
		cmd.process_group(0);
	}
	let _child = cmd.spawn()?;
	Ok(())
}

struct ProxyServer {
	client: Arc<TokioMutex<KernRpcClient<JsonEnvelopeCodec>>>,
}

impl McpServer for ProxyServer {
	fn server_name(&self) -> &str {
		"kern"
	}
	fn server_version(&self) -> &str {
		env!("CARGO_PKG_VERSION")
	}

	fn tools_list(&self) -> Vec<ToolSchema> {
		let client = self.client.clone();
		let res = crate::llm::block_on_in_place(async move {
			let c = client.lock().await;
			c.list_tools(trnsprt::kern_rpc::ListToolsReq {}).await
		});
		match res {
			Some(Ok(r)) => r
				.tools
				.into_iter()
				.filter_map(|v| serde_json::from_value(v).ok())
				.collect(),
			_ => crate::mcp::tools::typed_tool_schemas(),
		}
	}

	fn call_tool(&self, name: &str, args: &serde_json::Value) -> Result<ToolResult, McpError> {
		let client = self.client.clone();
		let req = CallToolReq {
			name: name.to_string(),
			args: args.clone(),
		};
		let res = crate::llm::block_on_in_place(async move {
			let first = {
				let c = client.lock().await;
				c.call_tool(req.clone()).await
			};
			match first {
				Ok(r) => Ok(r),
				// The daemon hot-reloads by handing its socket to a successor,
				// which severs established connections. Reconnect (riding out
				// the successor's graph load) and retry once. Safe to re-issue:
				// ingest is content-addressed and deduped, queries are reads.
				Err(_) => {
					let fresh = attach_with_retry(40, 250).await?;
					let res = fresh.call_tool(req).await;
					*client.lock().await = fresh;
					res
				}
			}
		})
		.ok_or_else(|| McpError::Rpc {
			code: -32000,
			message: "kern_rpc call_tool: no tokio runtime".to_string(),
		})?
		.map_err(|e| McpError::Rpc {
			code: -32000,
			message: format!("kern_rpc call_tool: {e}"),
		})?;

		Ok(crate::mcp::value_to_tool_result(&res.envelope))
	}

	fn extra_capabilities(&self) -> serde_json::Value {
		// Must match the standalone server so a client probing capabilities
		// can't tell the two apart.
		serde_json::json!({"resources": {}, "prompts": {}})
	}
}

enum StandaloneEntry {
	Own(crate::base::lock::WriterLock),
	Attach(Box<KernRpcClient<JsonEnvelopeCodec>>),
	Refuse(String),
}

// The standalone fallback is the one long-lived writer with nothing watching it.
// It loads the graph once and flushes its own snapshot for hours, so a second
// one — or one beside a daemon that never answered the attach window — ends with
// the loser's whole graph landing last. It also has no socket of its own, so a
// sibling standalone is invisible to every probe; the writer lock is the only
// thing that can see it. Claim before the load, exactly as the lock requires.
async fn claim_standalone(
	data_dir: &str,
	endpoint: &Endpoint,
	retries: u32,
	delay: std::time::Duration,
) -> StandaloneEntry {
	let held = match crate::base::lock::acquire(data_dir, "mcp-standalone") {
		Ok(l) => return StandaloneEntry::Own(l),
		Err(e) => e,
	};
	// The likeliest holder is the daemon this process just spawned, up at last
	// but after the attach window closed. Proxying to it is strictly better than
	// dying, so spend one more window before refusing.
	match KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(endpoint, retries, delay)
		.await
	{
		Ok(c) => StandaloneEntry::Attach(Box::new(c)),
		Err(_) => StandaloneEntry::Refuse(held.to_string()),
	}
}

async fn run_standalone(cfg: &crate::config::Config) {
	let _writer_lock = match claim_standalone(
		&cfg.data_dir,
		&Endpoint::kern(),
		40,
		std::time::Duration::from_millis(250),
	)
	.await
	{
		StandaloneEntry::Own(l) => l,
		StandaloneEntry::Attach(client) => return run_proxy(*client).await,
		StandaloneEntry::Refuse(who) => {
			eprintln!("kern mcp: {who}");
			eprintln!(
				"  refusing to serve standalone — a second whole-graph writer overwrites the first"
			);
			std::process::exit(1);
		}
	};
	let g = Arc::new(StdRwLock::new(load_graph(cfg)));
	let llm_client = super::server_llm_client(cfg, cfg.reason_url(), &cfg.reason.model);
	// Long-lived writer: same stale-flush guard as the daemon — never overwrite
	// a graph another process grew on disk with a staler snapshot.
	let save_g = g.clone();
	let save_cfg = cfg.clone();
	let save_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
		super::save_graph_guarded(&save_g, &save_cfg);
	});
	let q = Arc::new(crate::tick::queue::Queue::new(512));
	let defer: crate::ingest::worker::DeferQuestionsFn = {
		let defer_q = q.clone();
		Arc::new(move |entity_id: &str| {
			let _ = defer_q.enqueue(crate::tick::queue::task_extra(
				crate::tick::queue::TaskKind::SeedQuestions,
				"",
				entity_id,
			));
		})
	};
	let defer_contradiction: crate::ingest::worker::DeferContradictionFn = {
		let contra_q = q.clone();
		Arc::new(move |kern_id: &str, reason_id: &str| {
			let _ = contra_q.enqueue(crate::tick::queue::task_extra(
				crate::tick::queue::TaskKind::ClassifyContradiction,
				kern_id,
				reason_id,
			));
		})
	};
	let worker = Arc::new(crate::ingest::Worker::new(
		g.clone(),
		llm_client.clone(),
		Some(defer),
		Some(defer_contradiction),
		Some(save_fn.clone()),
	));

	let tick_llm: crate::tick::tasks::LlmFunc = Arc::new(llm_client.complete_func());
	let tick_embed: crate::tick::tasks::EmbedFunc = {
		let c = llm_client.clone();
		Arc::new(move |text: &str| -> Result<Vec<f32>, String> {
			let c = c.clone();
			let text = text.to_string();
			match tokio::runtime::Handle::try_current() {
				Ok(h) => {
					let result = std::thread::scope(|_| h.block_on(c.embed(&text)));
					result.map_err(|e: crate::llm::LlmError| e.to_string())
				}
				Err(_) => Err("no runtime".to_string()),
			}
		})
	};
	crate::tick::start(
		q.clone(),
		g.clone(),
		crate::tick::TickContext {
			llm: Some(tick_llm),
			embed: Some(tick_embed),
			broadcast_q: None,
			gnn_cfg: cfg.gnn.into(),
			tick_cfg: cfg.tick,
			heat_cfg: cfg.heat,
		},
	);

	let server = crate::mcp::Server {
		graph: g,
		worker,
		llm: Some(llm_client),
		save_fn,
		task_q: Some(q),
		cfg: Arc::new(cfg.clone()),
		broadcast_pulse: None,
		last_activity: Arc::new(std::sync::atomic::AtomicU64::new(
			crate::base::util::now_ms(),
		)),
	};
	server.run_stdio();
}

// Idempotent: inserts only absent entries, never touches existing keys.
pub(crate) fn ensure_mcp_registered(cwd: &std::path::Path) {
	let mcp_path = cwd.join(".mcp.json");

	let raw = std::fs::read_to_string(&mcp_path).unwrap_or_else(|_| "{}".to_string());
	let mut root: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));

	let wanted: &[(&str, serde_json::Value)] = &[(
		"kern",
		serde_json::json!({"command": "kern", "args": ["mcp"]}),
	)];

	let servers = root.as_object_mut().map(|obj| {
		obj
			.entry("mcpServers")
			.or_insert_with(|| serde_json::json!({}))
	});

	let Some(servers) = servers.and_then(|s| s.as_object_mut()) else {
		tracing::warn!(target: "kern.mcp", "ensure_mcp_registered: mcpServers is not an object");
		return;
	};

	let mut changed = false;
	for (name, entry) in wanted {
		if !servers.contains_key(*name) {
			servers.insert(name.to_string(), entry.clone());
			changed = true;
		}
	}

	if !changed {
		return;
	}

	match serde_json::to_string_pretty(&root) {
		Ok(json) => {
			if let Err(e) = std::fs::write(&mcp_path, json) {
				tracing::warn!(target: "kern.mcp", error = %e, "ensure_mcp_registered: write failed");
			} else {
				tracing::info!(
						target: "kern.mcp",
						path = %mcp_path.display(),
						"registered kern MCP server in .mcp.json"
				);
			}
		}
		Err(e) => {
			tracing::warn!(target: "kern.mcp", error = %e, "ensure_mcp_registered: serialize failed")
		}
	}
}

// Item 9's serving half for the one process the RPC route cannot help: the
// standalone fallback has no daemon to hand the write to, so its only correct
// answers are "I own the dir" or "I do not start".
#[cfg(all(test, unix))]
mod standalone_tests {
	use super::*;
	use std::sync::Arc;
	use std::time::Duration;
	use trnsprt::typed::{bind_kern_listener, BindOutcome};

	fn scratch_endpoint(tag: &str) -> Endpoint {
		let dir = std::env::temp_dir().join(format!(
			"kern-standalone-{}-{}-{tag}",
			std::process::id(),
			crate::base::util::now_ms()
		));
		std::fs::create_dir_all(&dir).expect("scratch dir");
		Endpoint::Unix(dir.join("kern.sock"))
	}

	async fn serving(endpoint: &Endpoint) {
		let BindOutcome::Bound(listener) = bind_kern_listener(endpoint).await.expect("bind") else {
			panic!("scratch endpoint already bound");
		};
		let handler = crate::rpc::kern_rpc_server::KernRpcHandler::new(
			Arc::new(crate::test_support::mcp_server()),
			Arc::new(tokio::sync::Notify::new()),
		);
		tokio::spawn(crate::rpc::kern_rpc_server::serve_kern_rpc_loop(
			listener, handler,
		));
	}

	#[tokio::test(flavor = "multi_thread")]
	async fn an_unclaimed_data_dir_is_owned_before_the_graph_is_read() {
		let dir = tempfile::tempdir().unwrap();
		let out = claim_standalone(
			dir.path().to_str().unwrap(),
			&scratch_endpoint("free"),
			1,
			Duration::ZERO,
		)
		.await;
		match out {
			StandaloneEntry::Own(l) => assert!(l.path().ends_with(crate::base::lock::LOCK_FILE)),
			_ => panic!("nothing holds the dir — the standalone server is its writer"),
		}
	}

	// The defect this closes: a sibling standalone holds the dir and serves no
	// socket, so nothing but the lock can see it. Booting anyway means two whole
	// graphs in memory and the loser's flush landing last.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_held_dir_with_nothing_serving_refuses_rather_than_writing_beside_it() {
		let dir = tempfile::tempdir().unwrap();
		let d = dir.path().to_str().unwrap();
		let _sibling = crate::base::lock::acquire(d, "mcp-standalone").expect("sibling claims it");

		let out = claim_standalone(d, &scratch_endpoint("held"), 1, Duration::ZERO).await;
		match out {
			StandaloneEntry::Refuse(who) => assert!(
				who.contains("mcp-standalone") && who.contains(&std::process::id().to_string()),
				"the refusal names the writer already there: {who}"
			),
			StandaloneEntry::Own(_) => panic!("became a second whole-graph writer"),
			StandaloneEntry::Attach(_) => panic!("attached to a socket nobody bound"),
		}
	}

	// And the cost of that refusal is bounded: the usual holder is the daemon
	// this process spawned, late to bind. Proxying to it beats dying.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_held_dir_whose_holder_answers_is_proxied_to_instead() {
		let dir = tempfile::tempdir().unwrap();
		let d = dir.path().to_str().unwrap();
		let _daemon = crate::base::lock::acquire(d, "daemon").expect("the daemon claims it");
		let ep = scratch_endpoint("late");
		serving(&ep).await;

		let out = claim_standalone(d, &ep, 5, Duration::from_millis(20)).await;
		assert!(
			matches!(out, StandaloneEntry::Attach(_)),
			"a holder that answers gets the traffic, not a refusal"
		);
	}
}

#[cfg(test)]
mod ensure_mcp_tests {
	use super::*;

	#[test]
	fn writes_kern_entry_when_file_absent() {
		let dir = tempfile::tempdir().unwrap();
		ensure_mcp_registered(dir.path());
		let raw = std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap();
		let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
		assert_eq!(v["mcpServers"]["kern"]["command"], "kern");
		assert_eq!(v["mcpServers"]["kern"]["args"][0], "mcp");
	}

	#[test]
	fn preserves_existing_keys_and_is_idempotent() {
		let dir = tempfile::tempdir().unwrap();
		let mcp = dir.path().join(".mcp.json");
		std::fs::write(&mcp, r#"{"mcpServers":{"other":{"command":"other"}}}"#).unwrap();

		ensure_mcp_registered(dir.path());

		let raw = std::fs::read_to_string(&mcp).unwrap();
		let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
		assert_eq!(v["mcpServers"]["other"]["command"], "other");
		assert_eq!(v["mcpServers"]["kern"]["command"], "kern");

		let before = std::fs::read_to_string(&mcp).unwrap();
		ensure_mcp_registered(dir.path());
		let after = std::fs::read_to_string(&mcp).unwrap();
		assert_eq!(before, after, "idempotent: file unchanged on second call");
	}

	#[test]
	fn does_not_overwrite_existing_custom_entries() {
		let dir = tempfile::tempdir().unwrap();
		let mcp = dir.path().join(".mcp.json");
		std::fs::write(
			&mcp,
			r#"{"mcpServers":{"kern":{"command":"custom","args":["x"]}}}"#,
		)
		.unwrap();

		ensure_mcp_registered(dir.path());

		let raw = std::fs::read_to_string(&mcp).unwrap();
		let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
		assert_eq!(v["mcpServers"]["kern"]["command"], "custom");
	}
}
