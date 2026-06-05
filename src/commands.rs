mod admin;
mod graph_ops;
mod ingest_cmd;
mod mcp_cmd;
mod query;
mod reembed;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::base::graph::GraphGnn;
use crate::base::locks::read_recovered;
use crate::base::search::find_entity;
use crate::base::types::Kern;
use crate::base::util::{short_id, truncate};

#[derive(Parser)]
#[command(name = "kern", version, about = "Self-organizing knowledge graph")]
pub struct Cli {
	#[command(subcommand)]
	pub command: Option<Commands>,

	/// Run the long-lived daemon (tick worker + MCP + kern_rpc) for this cwd.
	#[arg(short = 'd', long)]
	pub daemon: bool,

	/// Serve MCP over HTTP/SSE on this address instead of stdio (e.g. 127.0.0.1:7777).
	#[arg(long, default_value = "")]
	pub mcp_addr: String,

	/// Serve MCP over stdio (for direct embedding in an MCP client).
	#[arg(long)]
	pub mcp_stdio: bool,

	/// Embedding endpoint (Ollama-compatible). Overrides config.
	#[arg(long, default_value = crate::config::DEFAULT_EMBED_URL)]
	pub embed_url: String,

	/// Embedding model name. Overrides config.
	#[arg(long, default_value = crate::config::DEFAULT_EMBED_MODEL)]
	pub embed_model: String,

	/// Reasoning/distillation endpoint (Ollama-compatible). Overrides config.
	#[arg(long, default_value = "")]
	pub reason_url: String,

	/// Reasoning/distillation model name. Overrides config.
	#[arg(long, default_value = "")]
	pub reason_model: String,
}

#[derive(Subcommand)]
pub enum Commands {
	/// Add text (or a --file) to the graph; distills into typed claims.
	Ingest {
		text: Vec<String>,
		#[arg(long)]
		file: Option<String>,
		#[arg(long)]
		no_llm: bool,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
		#[arg(long)]
		reason_url: Option<String>,
		#[arg(long)]
		reason_model: Option<String>,
	},
	/// Search the graph; prints scored thoughts (+ optional LLM --answer).
	Query {
		text: String,
		#[arg(long, default_value = "hybrid")]
		mode: String,
		#[arg(long)]
		answer: bool,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
		#[arg(long)]
		reason_url: Option<String>,
		#[arg(long)]
		reason_model: Option<String>,
	},
	/// Raw vector + lexical search; print the top-k hits.
	Search {
		text: String,
		#[arg(long, default_value = "5")]
		k: usize,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	/// Re-embed the whole graph with the configured embedding model (run after
	/// changing `[embed] model`; stop the daemon first).
	Reembed {
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	/// Print a single thought by id (rehydrates from the cold store).
	Get {
		id: String,
	},
	/// List all thoughts in the graph.
	List,
	/// Remove a thought and cascade its edges (Facts are immune).
	Forget {
		id: String,
	},
	/// Create a reason edge between two thoughts (LLM writes the reason if blank).
	Link {
		from: String,
		to: String,
		#[arg(long, default_value = "")]
		reason: String,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
		#[arg(long)]
		reason_url: Option<String>,
		#[arg(long)]
		reason_model: Option<String>,
	},
	/// Print graph stats: thought/edge counts, tick heat, purpose.
	Health,
	/// Set or read the root purpose.
	Purpose {
		text: String,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	/// Down-weight the edges along a bad retrieval path (learn from a miss).
	Degrade {
		id: String,
	},
	/// Add or remove data-type descriptors.
	Descriptor {
		#[command(subcommand)]
		action: DescriptorAction,
	},
	/// List known gossip peers.
	Peers,
	/// Import a kern store from a directory into this graph.
	Register {
		path: String,
	},
	/// Inspect kerns that have no name yet.
	Unnamed {
		#[command(subcommand)]
		action: UnnamedAction,
	},
	/// Run the MCP server over stdio (attaches to or spawns the daemon).
	Mcp,
	/// Quantize a kern store's vectors (none | int8) into a new directory.
	Compress {
		src: String,
		#[arg(long, default_value = "int8")]
		mode: String,
		#[arg(long)]
		out: Option<String>,
	},
	/// Run a timed self-improvement hunt (feature-gated).
	#[cfg(feature = "hunt")]
	Hunt {
		#[arg(long, default_value = "60")]
		secs: u64,
	},
}

#[derive(Subcommand)]
pub enum DescriptorAction {
	Add { name: String, description: String },
	Rm { name: String },
}

#[derive(Subcommand)]
pub enum UnnamedAction {
	List,
}

pub(crate) fn load_graph(cfg: &crate::config::Config) -> GraphGnn {
	let mut g = match crate::base::persist::load_dir(&cfg.data_dir) {
		Ok(g) => g,
		Err(_) => {
			let mut g = GraphGnn::new();
			g.data_dir = cfg.data_dir.clone();
			g
		}
	};
	g.set_max_loaded_kerns(cfg.graph.max_kerns);
	g
}

pub(crate) fn save_graph(g: &GraphGnn) {
	if let Err(e) = crate::base::persist::save_all(g) {
		eprintln!("save: {e}");
	}
}

pub(crate) fn with_graph(cfg: &crate::config::Config, f: impl FnOnce(&mut GraphGnn)) {
	let mut g = load_graph(cfg);
	f(&mut g);
	save_graph(&g);
}

pub(crate) fn resolve<'a>(arg: &'a Option<String>, fallback: &'a str) -> &'a str {
	arg.as_deref().unwrap_or(fallback)
}

pub(crate) fn build_llm(
	embed_url: &str,
	embed_model: &str,
	embed_key: &str,
	reason_url: &str,
	reason_model: &str,
	reason_key: &str,
) -> crate::llm::Client {
	crate::llm::Client::new(
		reason_url,
		reason_model,
		reason_key,
		embed_url,
		embed_model,
		embed_key,
	)
}

pub(crate) fn find_entity_by_prefix(
	g: &GraphGnn,
	id: &str,
) -> Option<(crate::base::types::Entity, String)> {
	if let Some(pair) = find_entity(g, id) {
		return Some(pair);
	}
	for k in g.all() {
		for t in k.entities.values() {
			if t.id.starts_with(id) {
				return Some((t.clone(), k.id.clone()));
			}
		}
	}
	None
}

pub(crate) fn print_kern(kern: &Kern, g: &GraphGnn, depth: usize) {
	let indent = "  ".repeat(depth);
	let label = if kern.purpose_text.is_empty() {
		"[unnamed]".to_string()
	} else {
		kern.purpose_text.clone()
	};
	println!(
		"{}kern:{}  thoughts:{}  reasons:{}",
		indent,
		label,
		kern.entities.len(),
		kern.reasons.len(),
	);
	for t in kern.entities.values() {
		println!(
			"{}  [{}] {}",
			indent,
			short_id(&t.id),
			truncate(&t.text(), 72),
		);
	}
	for child_id in &kern.children {
		if let Some(child) = g.kerns.get(child_id) {
			print_kern(child, g, depth + 1);
		}
	}
}

pub async fn dispatch(cmd: Commands, cfg: &crate::config::Config) {
	match cmd {
		Commands::Ingest {
			text,
			file,
			no_llm,
			embed_url,
			embed_model,
			reason_url,
			reason_model,
		} => {
			ingest_cmd::cmd_ingest(
				cfg,
				text,
				file,
				no_llm,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
				resolve(&reason_url, &cfg.reason.url),
				resolve(&reason_model, &cfg.reason.model),
			)
			.await
		}

		Commands::Query {
			text,
			mode,
			answer,
			embed_url,
			embed_model,
			reason_url,
			reason_model,
		} => {
			query::cmd_query(
				cfg,
				&text,
				&mode,
				answer,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
				resolve(&reason_url, &cfg.reason.url),
				resolve(&reason_model, &cfg.reason.model),
			)
			.await
		}

		Commands::Search {
			text,
			k,
			embed_url,
			embed_model,
		} => {
			query::cmd_search(
				cfg,
				&text,
				k,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
			)
			.await
		}

		Commands::Reembed {
			embed_url,
			embed_model,
		} => {
			reembed::cmd_reembed(
				cfg,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
			)
			.await
		}

		Commands::Get { id } => graph_ops::cmd_get(cfg, &id),
		Commands::List => graph_ops::cmd_list(cfg),
		Commands::Forget { id } => graph_ops::cmd_forget(cfg, &id),

		Commands::Link {
			from,
			to,
			reason,
			embed_url,
			embed_model,
			reason_url,
			reason_model,
		} => {
			graph_ops::cmd_link(
				cfg,
				&from,
				&to,
				&reason,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
				resolve(&reason_url, &cfg.reason.url),
				resolve(&reason_model, &cfg.reason.model),
			)
			.await
		}

		Commands::Health => admin::cmd_health(cfg),

		Commands::Purpose {
			text,
			embed_url,
			embed_model,
		} => {
			admin::cmd_purpose(
				cfg,
				&text,
				resolve(&embed_url, &cfg.embed.url),
				resolve(&embed_model, &cfg.embed.model),
			)
			.await
		}

		Commands::Degrade { id } => graph_ops::cmd_degrade(cfg, &id),
		Commands::Descriptor { action } => admin::cmd_descriptor(cfg, action),
		Commands::Peers => admin::cmd_peers(),
		Commands::Register { path } => admin::cmd_register(cfg, &path),
		Commands::Unnamed { action } => admin::cmd_unnamed(cfg, action),
		Commands::Mcp => mcp_cmd::cmd_mcp(cfg).await,
		Commands::Compress { src, mode, out } => admin::cmd_compress(&src, &mode, out.as_deref()),
		#[cfg(feature = "hunt")]
		Commands::Hunt { secs } => {
			// Run the full daemon (TCP MCP + tick worker) and bail after
			// `secs`. cmd_mcp is stdio-driven and would EOF immediately
			// when launched without an interactive parent.
			let cfg = cfg.clone();
			let cli = Cli {
				command: None,
				daemon: true,
				mcp_addr: String::new(),
				mcp_stdio: false,
				embed_url: crate::config::DEFAULT_EMBED_URL.to_string(),
				embed_model: crate::config::DEFAULT_EMBED_MODEL.to_string(),
				reason_url: String::new(),
				reason_model: String::new(),
			};
			tokio::select! {
				_ = run_server(&cli, &cfg) => {}
				_ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {
					eprintln!("kern hunt: {secs}s elapsed, exiting");
				}
			}
		}
	}
}

pub async fn run_server(cli: &Cli, cfg: &crate::config::Config) {
	if let Some(j) = journal::global() {
		j.set_max_bytes(cfg.journal.max_today_bytes);
	}
	// Effective reason endpoint: CLI flag wins when set, else fall back to the
	// `[reason]` config section (which itself falls back to `[embed]`). Without
	// this the daemon ignored configured reasoning entirely, so distillation /
	// edge-proposal were silently disabled unless `--reason-url` was passed.
	let reason_url = if cli.reason_url.is_empty() {
		cfg.reason_url().to_string()
	} else {
		cli.reason_url.clone()
	};
	let reason_model = if cli.reason_model.is_empty() {
		cfg.reason.model.clone()
	} else {
		cli.reason_model.clone()
	};
	// The daemon embeds from its CONFIG, not the CLI flags. The `--daemon`
	// dispatch hardcodes cli.embed_* to DEFAULT_EMBED_* (a non-empty constant),
	// so reading them here ignored `[embed]` in kern.toml — the daemon embedded
	// with the default model even when the graph was built with another, a
	// dimension mismatch that makes every cosine degenerate. Use cfg.embed.
	let llm_client = build_llm(
		&cfg.embed.url,
		&cfg.embed.model,
		&cfg.embed.key,
		&reason_url,
		&reason_model,
		cfg.reason_key(),
	);

	let llm_fn: Option<crate::ingest::LlmFunc> = if !reason_url.is_empty() {
		Some(Arc::new(llm_client.complete_func()))
	} else {
		None
	};

	// Keep the embedding model resident. Ollama unloads it after ~5 min idle,
	// and the OpenAI-compat /v1 endpoint kern uses ignores `keep_alive`, so the
	// next query pays a multi-second cold reload (~7s for qwen3-embedding here).
	// A tiny embed every 4 min re-touches the model so retrieval stays warm
	// (~0.3s). Cheap and self-contained — no dependency on OLLAMA_KEEP_ALIVE.
	{
		let warm = llm_client.clone();
		tokio::spawn(async move {
			let mut tick = tokio::time::interval(std::time::Duration::from_secs(240));
			loop {
				tick.tick().await;
				let _ = warm.embed("kern-keepalive").await;
			}
		});
	}

	let tick_llm: crate::tick::tasks::LlmFunc = Arc::new(llm_client.complete_func());
	let tick_embed: crate::tick::tasks::EmbedFunc = {
		let c = llm_client.clone();
		Arc::new(move |text: &str| -> Result<Vec<f64>, String> {
			let c = c.clone();
			let text = text.to_string();
			match tokio::runtime::Handle::try_current() {
				Ok(h) => {
					// `block_in_place` hands this worker thread back to the
					// runtime while we block, so `block_on` is legal here.
					// Plain `block_on` on a runtime worker panics with
					// "Cannot start a runtime from within a runtime".
					let result =
						tokio::task::block_in_place(|| h.block_on(c.embed(&text)));
					result.map_err(|e: crate::llm::LlmError| e.to_string())
				}
				Err(_) => Err("no runtime".to_string()),
			}
		})
	};

	let registry = Arc::new(crate::store::Registry::new());
	let entry = registry.open(
		std::path::Path::new(&cfg.data_dir),
		cfg,
		llm_client.clone(),
		llm_fn.clone(),
		Some(tick_llm),
		Some(tick_embed),
		None,
	);
	let g = entry.graph.clone();
	let worker = entry.worker.clone();
	let q = entry.tick_q.clone();
	let save_fn = entry.save_fn.clone();

	// Live graph viewer — a read-only web UI over the current graph. Localhost
	// by default (cfg.serve.viewer); empty disables it.
	if !cfg.serve.viewer.is_empty() {
		let vg = g.clone();
		let vaddr = cfg.serve.viewer.clone();
		let viewer_llm = llm_client.clone();
		tokio::spawn(async move {
			if let Err(e) = crate::viewer::run(vg, viewer_llm, &vaddr).await {
				tracing::warn!(target: "kern.viewer", error = %e, "graph viewer failed to start");
			}
		});
	}

	// Slice K — session mirror. Tails the shared journal `fork_*`
	// lifecycle events and ingests each new fork as a `Document` entity
	// with `Source::Session`. Skipped silently if the project's history
	// SQLite cannot be opened (e.g. read-only filesystem during tests).
	{
		use crate::ingest::session_mirror::{run, SessionMirror, WorkerSink};
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		match journal::History::open(&cwd) {
			Ok(history) => {
				if cfg.journal.retain_days > 0 {
					match history.retain_days(cfg.journal.retain_days) {
						Ok(n) if n > 0 => tracing::info!(
							target: "kern.journal",
							pruned = n,
							retain_days = cfg.journal.retain_days,
							"history.db pruned"
						),
						Ok(_) => {}
						Err(e) => tracing::warn!(
							target: "kern.journal",
							error = %e,
							"history.db prune failed"
						),
					}
				}
				let history = Arc::new(history);
				let sink = WorkerSink::new(worker.clone());
				let mut sm = SessionMirror::new(sink);
				sm.set_max_seen(cfg.ingest.session_mirror_max_seen);
				let mirror = Arc::new(tokio::sync::Mutex::new(sm));
				tokio::spawn(run(history, mirror, std::time::Duration::from_secs(2)));
			}
			Err(e) => {
				tracing::warn!(target: "kern.session_mirror", error = %e, "history open failed; session mirror disabled");
			}
		}
	}

	// Slice O — kern-side filesystem watcher. Off unless the project's
	// `[watcher]` section in `.relay/kern.toml` sets `enabled = true`.
	// Roots default to cwd when `enabled = true` but no roots are listed.
	if cfg.watcher.enabled {
		use crate::ingest::file_watcher::{run as run_file_watcher, KernFileWatcherSink};
		use watcher::IgnoreRules;
		let roots: Vec<std::path::PathBuf> = if cfg.watcher.roots.is_empty() {
			vec![std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))]
		} else {
			cfg.watcher.roots.iter().map(std::path::PathBuf::from).collect()
		};
		let ignore = IgnoreRules::from_roots(&roots);
		let sink = Arc::new(KernFileWatcherSink::new(worker.clone()));
		tokio::spawn(async move {
			if let Err(e) = run_file_watcher(roots, ignore, sink).await {
				tracing::warn!(target: "kern.file_watcher", error = %e, "watcher exited");
			}
		});
	}

	// Claude-Code memory: capture spool drain + recall digest writer.
	// Both file-mediated; off unless `[capture] enabled = true` in
	// `.relay/kern.toml`.
	if cfg.capture.enabled {
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

		// Capture drain: spool deltas -> distill -> enqueue -> archive.
		if let Some(llm_fn) = llm_fn.clone() {
			let spool = cwd.join(&cfg.capture.dir);
			let worker_c = worker.clone();
			let dedup = cfg.ingest.dedup_threshold;
			let poll = std::time::Duration::from_secs(cfg.capture.poll_secs);
			let done_retention =
				std::time::Duration::from_secs(cfg.capture.done_retention_secs);
			tokio::spawn(crate::ingest::capture_spool::run(
				spool, worker_c, llm_fn, dedup, poll, done_retention,
			));
		} else {
			tracing::warn!(
				target: "kern.capture",
				"capture enabled but no reason LLM configured; distillation disabled"
			);
		}

		// Digest writer: periodically snapshot purpose + hot thoughts.
		{
			let digest_path = cwd.join(&cfg.capture.digest_path);
			let g_digest = g.clone();
			let k = cfg.capture.digest_k;
			let min_trust = cfg.capture.digest_min_trust as f64;
			let token_budget = cfg.capture.digest_token_budget;
			let every = std::time::Duration::from_secs(cfg.capture.digest_secs);
			tokio::spawn(async move {
				loop {
					{
						let g = read_recovered(&g_digest);
						crate::retrieval::digest::write_digest(
							&g,
							&digest_path,
							k,
							min_trust,
							token_budget,
						);
					}
					tokio::time::sleep(every).await;
				}
			});
		}
	}

	// Federation: start the gossip node so this kern can share/receive
	// knowledge with peers. OFF by default (`[gossip] enabled`). When on, it
	// binds a TCP listener, runs heartbeat, and (optionally) LAN multicast
	// discovery to auto-peer with same-network nodes.
	if cfg.gossip.enabled {
		let network_id = {
			let g = read_recovered(&g);
			g.network_id.clone()
		};
		let node = crate::gossip::node::Node::new(&cfg.gossip.addr, &network_id, cfg.gossip.peers.clone());
		let deps = Arc::new(crate::gossip::handler::Deps {
			graph: g.clone(),
			node: node.clone(),
			queue: Some(q.clone()),
			save: Some(save_fn.clone()),
		});
		node.set_handler(crate::gossip::handler::new_handler(deps));
		match node.listen().await {
			Ok(addr) => {
				tracing::info!(target: "kern.gossip", addr = %addr, network = %network_id, "gossip listening");
				node.start_heartbeat();
				crate::gossip::handler::start_announce(node.clone(), g.clone());
				crate::gossip::handler::start_entity_sync(node.clone(), g.clone());
				if cfg.gossip.discovery {
					crate::gossip::discovery::start_broadcast(&node, cfg.gossip.discovery_port);
					crate::gossip::discovery::start_listen(&node, cfg.gossip.discovery_port);
				}
			}
			Err(e) => {
				tracing::warn!(target: "kern.gossip", error = %e, "gossip listen failed; federation disabled");
			}
		}
	}

	// Autonomous maintenance tick: drives self-compaction on a timer instead
	// of only when something calls `pulse()`. Each tick pulses the root
	// (heat decay + stigmergy GC of cold nodes) and re-enqueues clustering,
	// so an idle daemon still decays, merges, and evicts. `interval_secs = 0`
	// disables it.
	if cfg.tick.interval_secs > 0 {
		let g_tick = g.clone();
		let q_tick = q.clone();
		let every = std::time::Duration::from_secs(cfg.tick.interval_secs);
		tokio::spawn(async move {
			loop {
				tokio::time::sleep(every).await;
				let root_id = {
					let g = read_recovered(&g_tick);
					g.root.id.clone()
				};
				{
					let mut g = crate::base::locks::write_recovered(&g_tick);
					crate::tick::pulse::pulse(&q_tick, &mut g, &root_id, 1.0);
				}
				crate::tick::enqueue_all(&q_tick, &g_tick);
			}
		});
	}

	let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.ok();
		let _ = shutdown_tx.send(());
	});

	let mcp_server = Arc::new(crate::mcp::Server {
		graph: g.clone(),
		worker: worker.clone(),
		llm: Some(llm_client.clone()),
		save_fn: save_fn.clone(),
		task_q: Some(q.clone()),
		cfg: Arc::new(cfg.clone()),
	});

	// Singleton kern_rpc surface on the per-user `kern.sock` endpoint.
	// Bound synchronously here so an `AlreadyRunning` outcome (another
	// daemon owns the endpoint) can short-circuit run_server before any
	// other daemon scaffolding spins up. The accept loop runs in a
	// detached task; this function returns when ctrl-c arrives (daemon
	// mode) or when the repl exits (interactive mode).
	{
		let mem = Arc::new(std::sync::Mutex::new(crate::memory_service::MemoryService::new()));
		let handler = crate::rpc::KernRpcHandler::new(mcp_server.clone(), mem);
		let endpoint = trnsprt::typed::Endpoint::kern();
		match trnsprt::typed::bind_kern_listener(&endpoint).await {
			Ok(trnsprt::typed::BindOutcome::Bound(listener)) => {
				tracing::info!(
					target: "kern.kern_rpc",
					endpoint = %endpoint.display(),
					"listening"
				);
				tokio::spawn(crate::rpc::serve_kern_rpc_loop(listener, handler));
			}
			Ok(trnsprt::typed::BindOutcome::AlreadyRunning) => {
				eprintln!(
					"kern: another daemon already running at {} — exiting",
					endpoint.display()
				);
				return;
			}
			Err(e) => {
				tracing::error!(target: "kern.kern_rpc", error = %e, "bind failed");
				return;
			}
		}
	}

	if cli.mcp_stdio {
		mcp_server.run_stdio();
	} else {
		if !cli.mcp_addr.is_empty() {
			let mcp_addr = cli.mcp_addr.clone();
			let mcp_s = mcp_server.clone();
			tokio::spawn(async move {
				if let Err(e) = crate::mcp::sse::run_sse(mcp_s, &mcp_addr).await {
					eprintln!("mcp-sse: {e}");
				}
			});
		}

		if !cli.daemon {
			crate::repl::run(
				g.clone(),
				worker,
				llm_client,
				Some(q.clone()),
				cfg.ingest.dedup_threshold,
			)
			.await;
		} else {
			println!("kern running in daemon mode (ctrl-c to stop)");
			let _ = shutdown_rx.await;
		}
	}

	drop(q);

	eprintln!("shutting down...");
	{
		let g = read_recovered(&g);
		save_graph(&g);
	}
	eprintln!("done");
}
