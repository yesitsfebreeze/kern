mod admin;
mod graph_ops;
mod ingest_cmd;
mod mcp_cmd;
mod profile_cmd;
mod query;
mod reembed;

pub(crate) use mcp_cmd::ensure_mcp_registered;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::base::graph::GraphGnn;
use crate::base::locks::read_recovered;

const SELF_HEAL_BLOAT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "kern", version, about = "Self-organizing knowledge graph")]
pub struct Cli {
	#[command(subcommand)]
	pub command: Option<Commands>,

	#[arg(short = 'd', long)]
	pub daemon: bool,

	#[arg(long, default_value = "")]
	pub mcp_addr: String,

	#[arg(long)]
	pub mcp_stdio: bool,

	#[arg(long, default_value = crate::config::DEFAULT_EMBED_URL)]
	pub embed_url: String,

	#[arg(long, default_value = crate::config::DEFAULT_EMBED_MODEL)]
	pub embed_model: String,

	#[arg(long, default_value = "")]
	pub reason_url: String,

	#[arg(long, default_value = "")]
	pub reason_model: String,
}

impl Cli {
	fn daemon() -> Self {
		Cli {
			command: None,
			daemon: true,
			mcp_addr: String::new(),
			mcp_stdio: false,
			embed_url: crate::config::DEFAULT_EMBED_URL.to_string(),
			embed_model: crate::config::DEFAULT_EMBED_MODEL.to_string(),
			reason_url: String::new(),
			reason_model: String::new(),
		}
	}
}

#[derive(Subcommand)]
pub enum Commands {
	Ingest {
		text: Vec<String>,
		#[arg(long)]
		file: Option<String>,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
		#[arg(long)]
		reason_url: Option<String>,
		#[arg(long)]
		reason_model: Option<String>,
	},
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
	Search {
		text: String,
		#[arg(long, default_value = "5")]
		k: usize,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	Reembed {
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	Get {
		id: String,
	},
	List,
	Forget {
		id: String,
	},
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
	Health,
	Profile {
		#[arg(long, default_value = "what is this project about")]
		text: String,
		#[arg(long)]
		no_llm: bool,
	},
	Gc,
	Compact,
	Anchor {
		#[command(subcommand)]
		action: AnchorAction,
	},
	Degrade {
		id: String,
	},
	Descriptor {
		#[command(subcommand)]
		action: DescriptorAction,
	},
	Peers,
	Register {
		path: String,
	},
	Unnamed {
		#[command(subcommand)]
		action: UnnamedAction,
	},
	Mcp,
	Compress {
		src: String,
		#[arg(long, default_value = "int8")]
		mode: String,
		#[arg(long)]
		out: Option<String>,
	},
	Migrate {
		path: Option<String>,
	},
	Daemon,
}

#[derive(Subcommand)]
pub enum AnchorAction {
	Add {
		name: String,
		text: String,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	List,
	Remove {
		name: String,
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

pub(crate) fn apply_graph_config(g: &mut GraphGnn, cfg: &crate::config::GraphConfig) {
	g.set_max_loaded_kerns(cfg.max_kerns);
	g.set_disk_threshold(cfg.disk_threshold);
	if cfg.disk_threshold != crate::base::constants::KERN_CAP_DISABLED {
		g.rebuild_index();
	}
}

pub(crate) fn load_graph(cfg: &crate::config::Config) -> GraphGnn {
	let mut g = match crate::base::persist::load_dir(&cfg.data_dir) {
		Ok(g) => g,
		Err(_) => {
			let mut g = GraphGnn::new();
			g.data_dir = cfg.data_dir.clone();
			if let Ok(store) = crate::base::store::Store::open(&cfg.data_dir) {
				g.set_store(std::sync::Arc::new(store));
			}
			g
		}
	};
	apply_graph_config(&mut g, &cfg.graph);
	if let Some(lex) = g.lexical() {
		lex.set_bm25_params(cfg.retrieval.bm25_k1 as f32, cfg.retrieval.bm25_b as f32);
	}
	g
}

pub(crate) fn save_graph(g: &GraphGnn) {
	if let Err(e) = crate::base::persist::save_all(g) {
		eprintln!("save: {e}");
	}
}

pub(crate) fn reload_graph(cfg: &crate::config::Config, old: &GraphGnn) -> GraphGnn {
	match crate::base::persist::reload_from_disk(old) {
		Some(mut g) => {
			apply_graph_config(&mut g, &cfg.graph);
			if let Some(lex) = g.lexical() {
				lex.set_bm25_params(cfg.retrieval.bm25_k1 as f32, cfg.retrieval.bm25_b as f32);
			}
			g
		}
		None => load_graph(cfg),
	}
}

pub(crate) fn save_graph_guarded(
	graph: &std::sync::Arc<parking_lot::RwLock<GraphGnn>>,
	cfg: &crate::config::Config,
) {
	const FLUSH_RETRIES: u32 = 5;
	for attempt in 0..FLUSH_RETRIES {
		let (snapshot, expected) = {
			let g = read_recovered(graph);
			(
				crate::base::persist::snapshot_for_flush(&g),
				g.flushed_epoch(),
			)
		};
		let Some(snapshot) = snapshot else {
			return;
		};
		let outcome = crate::base::persist::flush_snapshot(&snapshot, expected);
		match outcome {
			Ok(crate::base::store::FlushOutcome::Flushed { epoch }) => {
				crate::base::locks::write_recovered(graph).set_flushed_epoch(epoch);
				return;
			}
			Ok(crate::base::store::FlushOutcome::RefusedStale {
				disk_epoch,
				expected,
			}) => {
				tracing::warn!(
					target: "kern.persist",
					disk_epoch,
					expected,
					attempt,
					data_dir = %cfg.data_dir,
					"refused to flush a stale snapshot — disk advanced under us (another writer); absorbing disk rows and retrying"
				);
				let mut w = crate::base::locks::write_recovered(graph);
				let Some(fresh) = crate::base::persist::reload_from_disk(&w) else {
					return;
				};
				let disk_epoch = fresh.flushed_epoch();
				crate::base::merge::absorb_graph(&mut w, fresh);
				w.set_flushed_epoch(disk_epoch);
			}
			Err(e) => {
				eprintln!("save: {e}");
				return;
			}
		}
	}
	tracing::warn!(
		target: "kern.persist",
		data_dir = %cfg.data_dir,
		"flush still refused after {FLUSH_RETRIES} absorb-and-retry rounds; unflushed rows stay in memory until the next snapshot"
	);
}

pub(crate) fn snapshot_if_dirty(
	graph: &SharedGraph,
	cfg: &crate::config::Config,
	last_snap_epoch: &mut u64,
) -> bool {
	let epoch = read_recovered(graph).mutation_epoch();
	if epoch == *last_snap_epoch {
		return false;
	}
	save_graph_guarded(graph, cfg);
	*last_snap_epoch = epoch;
	true
}

pub(crate) fn reconcile_if_stale(
	graph: &std::sync::Arc<parking_lot::RwLock<GraphGnn>>,
	cfg: &crate::config::Config,
) -> bool {
	let mut w = crate::base::locks::write_recovered(graph);
	let stale = match w.store() {
		Some(store) => store.read_epoch() > w.flushed_epoch(),
		None => false,
	};
	if stale {
		// Reload reusing the open store handle: load_graph would double-open the
		// LMDB env on a dir already open in this process.
		let fresh = reload_graph(cfg, &w);
		*w = fresh;
		tracing::info!(
			target: "kern.persist",
			data_dir = %cfg.data_dir,
			"store advanced under the daemon (external write); reloaded graph from disk"
		);
	}
	stale
}

fn maybe_self_heal_store(cfg: &crate::config::Config) {
	let data = std::path::Path::new(&cfg.data_dir).join("data.mdb");
	let len = std::fs::metadata(&data).map(|m| m.len()).unwrap_or(0);
	if len < SELF_HEAL_BLOAT_BYTES {
		return;
	}
	tracing::info!(target: "kern.startup", bytes = len, "data.mdb is bloated; self-healing (reap + compact)");

	// Drop the throwaway graph so its env handle releases before the compaction swap.
	{
		let mut g = load_graph(cfg);
		let (before, reaped, after) = g.gc_empty_kerns_counted();
		if reaped > 0 {
			save_graph(&g);
			eprintln!("kern: self-heal reaped {reaped} empty kerns ({before} -> {after})");
		}
	}
	match crate::base::store::compact_dir(&cfg.data_dir) {
		Ok((old, new)) => eprintln!(
			"kern: self-heal compacted data.mdb {} MiB -> {} MiB",
			old / (1024 * 1024),
			new / (1024 * 1024),
		),
		Err(e) => {
			tracing::warn!(target: "kern.startup", error = %e, "self-heal compaction skipped (store may be held by another process)");
		}
	}
}

pub(crate) fn with_graph<R>(cfg: &crate::config::Config, f: impl FnOnce(&mut GraphGnn) -> R) -> R {
	let mut g = load_graph(cfg);
	let out = f(&mut g);
	save_graph(&g);
	out
}

pub(crate) fn resolve<'a>(arg: &'a Option<String>, fallback: &'a str) -> &'a str {
	arg.as_deref().unwrap_or(fallback)
}

pub(crate) use crate::llm::{Client, Endpoint};

pub(crate) fn embed_fn(client: &Client) -> crate::types::EmbedFunc {
	let c = client.clone();
	std::sync::Arc::new(move |text: &str| -> Result<Vec<f32>, String> {
		let c = c.clone();
		let text = text.to_string();
		match crate::llm::block_on_in_place(c.embed(&text)) {
			Some(r) => r.map_err(|e| e.to_string()),
			None => Err("no runtime".to_string()),
		}
	})
}

// answer/embed are ALWAYS taken from config — embedding with any model but the
// graph's degenerates every cosine.
pub(crate) fn server_llm_client(
	cfg: &crate::config::Config,
	reason_url: &str,
	reason_model: &str,
) -> Client {
	Client::new(
		Endpoint::new(reason_url, reason_model, cfg.reason_key()),
		Endpoint::new(cfg.answer_url(), &cfg.answer.model, cfg.answer_key()),
		Endpoint::new(&cfg.embed.url, &cfg.embed.model, &cfg.embed.key),
	)
}

pub async fn dispatch(cmd: Commands, cfg: &crate::config::Config) {
	match cmd {
		Commands::Ingest {
			text,
			file,
			embed_url,
			embed_model,
			reason_url,
			reason_model,
		} => {
			ingest_cmd::cmd_ingest(
				cfg,
				text,
				file,
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
				query::QueryParams {
					text: &text,
					mode: &mode,
					answer,
					embed_url: resolve(&embed_url, &cfg.embed.url),
					embed_model: resolve(&embed_model, &cfg.embed.model),
					reason_url: resolve(&reason_url, &cfg.reason.url),
					reason_model: resolve(&reason_model, &cfg.reason.model),
				},
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
		Commands::Profile { text, no_llm } => profile_cmd::cmd_profile(cfg, &text, no_llm).await,
		Commands::Gc => admin::cmd_gc(cfg),
		Commands::Compact => admin::cmd_compact(cfg),

		Commands::Anchor { action } => admin::cmd_anchor(cfg, action).await,

		Commands::Degrade { id } => graph_ops::cmd_degrade(cfg, &id),
		Commands::Descriptor { action } => admin::cmd_descriptor(cfg, action),
		Commands::Peers => admin::cmd_peers(cfg),
		Commands::Register { path } => admin::cmd_register(cfg, &path),
		Commands::Unnamed { action } => admin::cmd_unnamed(cfg, action),
		Commands::Mcp => mcp_cmd::cmd_mcp(cfg).await,
		Commands::Compress { src, mode, out } => admin::cmd_compress(&src, &mode, out.as_deref()),
		Commands::Migrate { path } => {
			let dir = path.unwrap_or_else(|| cfg.data_dir.clone());
			match crate::base::migrate::migrate_dir(&dir) {
				Ok(r) => println!(
					"migrated {} kerns ({} entities) → {dir}/data.mdb (old .kern files left in place)",
					r.kerns, r.entities
				),
				Err(e) => eprintln!("migrate: {e}"),
			}
		}
		Commands::Daemon => {
			// main.rs intercepts Daemon first; this arm is kept as a fallthrough.
			run_server(&Cli::daemon(), cfg).await;
		}
	}
}

pub(crate) struct EngineHandle {
	pub server: std::sync::Arc<crate::mcp::Server>,
	pub task_q: std::sync::Arc<crate::tick::queue::Queue>,
	// Guarded persist closure: the shutdown flush never overwrites a grown disk.
	pub save_fn: std::sync::Arc<dyn Fn() + Send + Sync>,
}

pub(crate) async fn bootstrap(cli: &Cli, cfg: &crate::config::Config) -> EngineHandle {
	// Must run BEFORE any env opens: the compaction swaps data.mdb, and only
	// here — post kern.sock win, pre env open — is the dir held exclusively.
	maybe_self_heal_store(cfg);

	spawn_watchdog();
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
	// Embed/answer must come from cfg, never cli.embed_* — see server_llm_client.
	let llm_client = server_llm_client(cfg, &reason_url, &reason_model);

	let llm_fn: Option<crate::ingest::LlmFunc> = if !reason_url.is_empty() {
		Some(Arc::new(llm_client.complete_func()))
	} else {
		None
	};

	spawn_keepalive(&llm_client);

	// Gate like `llm_fn`: an ungated Some with no reason endpoint means infinite
	// no-op Name re-enqueue churn (do_cluster gates on `llm.is_some()`).
	let tick_llm: Option<crate::tick::tasks::LlmFunc> = if reason_url.is_empty() {
		None
	} else {
		Some(Arc::new(llm_client.complete_func()))
	};
	let tick_embed: crate::tick::tasks::EmbedFunc = embed_fn(&llm_client);

	let registry = Arc::new(crate::store::Registry::new());
	let shared_bq: Arc<parking_lot::RwLock<Option<crate::tick::tasks::BroadcastQuestionFunc>>> =
		Arc::new(parking_lot::RwLock::new(None));
	let bq_slot = shared_bq.clone();
	let broadcast_q_wrapper: crate::tick::tasks::BroadcastQuestionFunc =
		Arc::new(move |rid, from, vec, text| {
			if let Some(f) = bq_slot.read().as_ref() {
				f(rid, from, vec, text);
			}
		});
	let entry = registry.open(
		std::path::Path::new(&cfg.data_dir),
		cfg,
		llm_client.clone(),
		tick_llm,
		Some(tick_embed),
		Some(broadcast_q_wrapper),
	);
	let g = entry.graph.clone();
	let worker = entry.worker.clone();
	let q = entry.tick_q.clone();
	let save_fn = entry.save_fn.clone();

	{
		let (before, reaped, after) = crate::base::locks::write_recovered(&g).gc_empty_kerns_counted();
		if reaped > 0 {
			tracing::info!(
				target: "kern.startup",
				reaped,
				before,
				after,
				"reaped empty unnamed kerns"
			);
			eprintln!("kern: reaped {reaped} empty kerns ({before} -> {after})");
			// Persist via the guarded closure (not bare save_graph) so the epoch bump
			// stays tracked — else the next flush refuse-reloads its own reap.
			save_fn();
		}
	}

	let mcp_server = std::sync::Arc::new(crate::mcp::Server {
		graph: g.clone(),
		worker: worker.clone(),
		llm: Some(llm_client.clone()),
		save_fn: save_fn.clone(),
		task_q: Some(q.clone()),
		cfg: std::sync::Arc::new(cfg.clone()),
		cache: crate::retrieval::cache::QueryCache::shared(
			cfg.retrieval.query_cache_cap,
			cfg.retrieval.query_cache_theta,
		),
		broadcast_pulse: None,
	});

	spawn_file_watcher(cfg, &worker);

	spawn_capture(cfg, &worker, &llm_fn, &g);

	let (broadcast_pulse, broadcast_q) = start_gossip(cfg, &g, &q, &save_fn).await;
	if let Some(bq) = broadcast_q {
		*shared_bq.write() = Some(bq);
	}

	spawn_maintenance_tick(cfg, &g, &q, broadcast_pulse.clone());

	EngineHandle {
		server: mcp_server,
		task_q: q,
		save_fn,
	}
}

pub async fn run_server(cli: &Cli, cfg: &crate::config::Config) {
	{
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		ensure_mcp_registered(&cwd);
	}

	let h = bootstrap(cli, cfg).await;
	let q = h.task_q.clone();
	let mcp_server = h.server.clone();
	let save_fn = h.save_fn.clone();

	let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.ok();
		let _ = shutdown_tx.send(());
	});

	// kern.sock bound synchronously so `AlreadyRunning` short-circuits before more
	// scaffolding spins up.
	{
		let handler = crate::rpc::KernRpcHandler::new(mcp_server.clone());
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
					tracing::error!(target: "kern.mcp_sse", error = %e, "MCP-over-HTTP server exited");
				}
			});
		}

		println!("kern running in daemon mode (ctrl-c to stop)");
		let _ = shutdown_rx.await;
	}

	drop(q);

	eprintln!("shutting down...");
	// Shut down through the store's guarded closure so a stale daemon's final
	// flush can't wipe a graph the CLI grew on disk (the SIGTERM data-loss path).
	save_fn();
	eprintln!("done");
}

type SharedGraph = Arc<parking_lot::RwLock<GraphGnn>>;

// Force-exits if the async beat stalls ~30s (deadlock/starvation) so a peer can take the hub.
fn spawn_watchdog() {
	use std::sync::atomic::{AtomicU64, Ordering};
	let beat = Arc::new(AtomicU64::new(0));
	{
		let beat = beat.clone();
		tokio::spawn(async move {
			let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
			loop {
				tick.tick().await;
				beat.fetch_add(1, Ordering::Relaxed);
			}
		});
	}
	std::thread::Builder::new()
		.name("kern-watchdog".into())
		.spawn(move || {
			const CHECK_SECS: u64 = 5;
			const STALL_LIMIT: u32 = 6; // 6 * 5s = 30s of no async progress
			let mut last = 0u64;
			let mut stalls = 0u32;
			loop {
				std::thread::sleep(std::time::Duration::from_secs(CHECK_SECS));
				let now = beat.load(Ordering::Relaxed);
				if now == last {
					stalls += 1;
					if stalls >= STALL_LIMIT {
						eprintln!(
							"kern watchdog: async runtime stalled ~{}s (graph deadlock or worker starvation) — exiting so a peer can take the hub",
							u64::from(stalls) * CHECK_SECS
						);
						std::process::exit(101);
					}
				} else {
					stalls = 0;
					last = now;
				}
			}
		})
		.expect("spawn kern-watchdog thread");
}

// Ollama unloads after ~5 min idle and /v1 ignores `keep_alive`; ping every 4 min
// keeps both models resident.
fn spawn_keepalive(llm_client: &Client) {
	let warm = llm_client.clone();
	tokio::spawn(async move {
		use futures_util::StreamExt as _;
		let mut tick = tokio::time::interval(std::time::Duration::from_secs(240));
		loop {
			tick.tick().await;
			let embed = warm.embed("kern-keepalive");
			let answer = async {
				let mut gen = std::pin::pin!(warm.answer(crate::llm::AnswerParams {
					messages: vec![("user".to_string(), "warm".to_string())],
					stream: false,
					num_predict: Some(1),
				}));
				while gen.next().await.is_some() {}
			};
			let (_, _) = tokio::join!(embed, answer);
		}
	});
}

fn spawn_file_watcher(cfg: &crate::config::Config, worker: &Arc<crate::ingest::Worker>) {
	if !cfg.watcher.enabled {
		return;
	}
	use crate::ingest::file_watcher::{run as run_file_watcher, KernFileWatcherSink};
	use watcher::IgnoreRules;
	let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
	let roots = cfg.watcher.effective_roots(&cwd);
	let ignore = IgnoreRules::from_roots(&roots);
	let sink = Arc::new(KernFileWatcherSink::new(worker.clone()));
	tokio::spawn(async move {
		if let Err(e) = run_file_watcher(roots, ignore, sink).await {
			tracing::warn!(target: "kern.file_watcher", error = %e, "watcher exited");
		}
	});
}

fn spawn_capture(
	cfg: &crate::config::Config,
	worker: &Arc<crate::ingest::Worker>,
	llm_fn: &Option<crate::ingest::LlmFunc>,
	g: &SharedGraph,
) {
	if !cfg.capture.enabled {
		return;
	}
	let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

	if let Some(llm_fn) = llm_fn.clone() {
		let intake = cwd.join(&cfg.capture.dir);
		let worker_c = worker.clone();
		let dedup = cfg.ingest.dedup_threshold;
		let poll = std::time::Duration::from_secs(cfg.capture.poll_secs);
		let done_retention = std::time::Duration::from_secs(cfg.capture.done_retention_secs);
		tokio::spawn(crate::ingest::intake::run(
			intake,
			worker_c,
			llm_fn,
			dedup,
			poll,
			done_retention,
		));
	} else {
		tracing::warn!(
			target: "kern.capture",
			"capture: intake drain inactive — add a [reason] section to kern.toml to enable distillation; deltas will accumulate in .kern/capture/ and will be processed once the daemon restarts with a reason LLM configured"
		);
	}

	let digest_path = cwd.join(&cfg.capture.digest_path);
	let g_digest = g.clone();
	let k = cfg.capture.digest_k;
	let min_trust = cfg.capture.digest_min_trust;
	let token_budget = cfg.capture.digest_token_budget;
	let every = std::time::Duration::from_secs(cfg.capture.digest_secs);
	tokio::spawn(async move {
		loop {
			{
				let g = read_recovered(&g_digest);
				crate::retrieval::digest::write_digest(&g, &digest_path, k, min_trust, token_budget);
			}
			tokio::time::sleep(every).await;
		}
	});
}

type BroadcastPulseFn = Arc<dyn Fn(&str, f64) + Send + Sync>;

async fn start_gossip(
	cfg: &crate::config::Config,
	g: &SharedGraph,
	q: &Arc<crate::tick::queue::Queue>,
	save_fn: &Arc<dyn Fn() + Send + Sync>,
) -> (
	Option<BroadcastPulseFn>,
	Option<crate::tick::tasks::BroadcastQuestionFunc>,
) {
	if !cfg.gossip.enabled {
		return (None, None);
	}
	let network_id = {
		let g = read_recovered(g);
		g.network_id.clone()
	};
	let network_id = cfg.gossip.effective_network_id(&network_id);
	let node =
		crate::gossip::node::Node::new(&cfg.gossip.addr, &network_id, cfg.gossip.peers.clone());
	node.ledger.set_max_entries(cfg.graph.max_ledger_entries);
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
			crate::gossip::handler::start_delta_flush(node.clone(), g.clone());
			if cfg.gossip.discovery {
				crate::gossip::discovery::start_broadcast(&node, cfg.gossip.discovery_port);
				crate::gossip::discovery::start_listen(&node, cfg.gossip.discovery_port);
			}
			let pulse_node = node.clone();
			let broadcast_pulse: BroadcastPulseFn = Arc::new(move |kern_id: &str, strength: f64| {
				let stamp = crate::base::util::now_nanos();
				let msg = crate::gossip::types::GossipMessage {
					kind: crate::gossip::types::GossipKind::Pulse,
					id: format!("pulse-{}-{}", pulse_node.addr(), stamp),
					origin: pulse_node.addr(),
					payload: crate::gossip::types::GossipPayload::Pulse(crate::gossip::types::PulsePayload {
						kern_id: kern_id.to_string(),
						strength,
					}),
				};
				pulse_node.broadcast(msg);
			});
			let q_node = node.clone();
			let broadcast_q: crate::tick::tasks::BroadcastQuestionFunc =
				Arc::new(move |rid: &str, from_id: &str, rvec: &[f32], rtext: &str| {
					let stamp = crate::base::util::now_nanos();
					let msg = crate::gossip::types::GossipMessage {
						kind: crate::gossip::types::GossipKind::Question,
						id: format!("q-{}-{}", q_node.addr(), stamp),
						origin: q_node.addr(),
						payload: crate::gossip::types::GossipPayload::Question(
							crate::gossip::types::QuestionPayload {
								reason_id: rid.to_string(),
								from_id: from_id.to_string(),
								reason_vec: rvec.to_vec(),
								question_text: rtext.to_string(),
							},
						),
					};
					q_node.broadcast(msg);
				});
			(Some(broadcast_pulse), Some(broadcast_q))
		}
		Err(e) => {
			tracing::warn!(target: "kern.gossip", error = %e, "gossip listen failed; federation disabled");
			(None, None)
		}
	}
}

fn spawn_maintenance_tick(
	cfg: &crate::config::Config,
	g: &SharedGraph,
	q: &Arc<crate::tick::queue::Queue>,
	broadcast_pulse: Option<crate::mcp::PulseBroadcast>,
) {
	if cfg.tick.interval_secs == 0 {
		return;
	}
	let g_tick = g.clone();
	let q_tick = q.clone();
	let cfg_tick = cfg.clone();
	let every = std::time::Duration::from_secs(cfg.tick.interval_secs);
	let mut last_snap_epoch = read_recovered(g).mutation_epoch();
	tokio::spawn(async move {
		loop {
			tokio::time::sleep(every).await;
			// Must run before the tick mutates and persists: adopt concurrent CLI
			// writes, or per-kern persist writes stale kerns over newer disk rows.
			reconcile_if_stale(&g_tick, &cfg_tick);
			let root_id = {
				let g = read_recovered(&g_tick);
				g.root.id.clone()
			};
			{
				let mut g = crate::base::locks::write_recovered(&g_tick);
				crate::tick::pulse::pulse(&q_tick, &mut g, &root_id, 1.0);
			}
			if let Some(broadcast) = &broadcast_pulse {
				broadcast(&root_id, 1.0);
			}
			crate::tick::enqueue_all(&q_tick, &g_tick);
			// Bound the crash-loss window for mutations whose event-driven save
			// never ran (crash pre-Persist, SIGTERM pre-flush) to one interval.
			snapshot_if_dirty(&g_tick, &cfg_tick, &mut last_snap_epoch);
		}
	});
}

#[cfg(test)]
mod entry_point_tests {
	use super::Commands;

	#[test]
	fn daemon_subcommand_exists() {
		let _ = Commands::Daemon;
	}

	// LMDB forbids double-opening one env per process, so the "external writer"
	// commits THROUGH the daemon graph's own store handle — same divergence.
	#[cfg(test)]
	fn commit_extra_kern_via_store(
		g: &std::sync::Arc<parking_lot::RwLock<super::GraphGnn>>,
		kern: crate::base::types::Kern,
	) {
		use crate::base::locks::read_recovered;
		let gg = read_recovered(g);
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

	#[test]
	fn save_graph_guarded_absorbs_external_commit_and_keeps_unflushed_rows() {
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::locks::{read_recovered, write_recovered};
		use crate::base::types::{mk_entity, EntityKind, Kern};

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};

		let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
		assert_eq!(
			read_recovered(&g).flushed_epoch(),
			0,
			"fresh load at epoch 0"
		);

		let root_id = read_recovered(&g).root.id.clone();
		commit_extra_kern_via_store(&g, Kern::new("cli-kern", &root_id));

		let mut ram = Kern::new("ram-kern", &root_id);
		ram.entities.insert(
			"e1".into(),
			mk_entity("e1", "unflushed row", 1.0, EntityKind::Fact),
		);
		write_recovered(&g)
			.kerns
			.insert("ram-kern".to_string(), ram);

		super::save_graph_guarded(&g, &cfg);

		assert!(
			read_recovered(&g).loaded("cli-kern").is_some(),
			"the externally committed kern was absorbed instead of ignored"
		);
		assert!(
			read_recovered(&g).loaded("ram-kern").is_some(),
			"the unflushed in-memory kern survived the refused flush"
		);
		assert!(
			read_recovered(&g).flushed_epoch() >= 2,
			"the daemon adopted the advanced on-disk epoch and flushed past it"
		);
		// Read disk back through the same store handle (no second env open).
		let store = read_recovered(&g).store().unwrap();
		assert!(
			store.load_one_kern("cli-kern").unwrap().is_some(),
			"the externally committed kern survives on disk"
		);
		assert!(
			store.load_one_kern("ram-kern").unwrap().is_some(),
			"the unflushed in-memory kern reached disk on the retry flush"
		);
	}

	#[test]
	fn reconcile_if_stale_reloads_only_when_the_store_advanced() {
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::locks::read_recovered;
		use crate::base::types::Kern;

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};

		let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
		assert!(
			!super::reconcile_if_stale(&g, &cfg),
			"nothing committed yet -> no reload"
		);

		let root_id = read_recovered(&g).root.id.clone();
		commit_extra_kern_via_store(&g, Kern::new("late", &root_id));

		assert!(
			super::reconcile_if_stale(&g, &cfg),
			"store advanced -> reload"
		);
		assert!(
			read_recovered(&g).loaded("late").is_some(),
			"adopted the new kern"
		);
		assert!(
			!super::reconcile_if_stale(&g, &cfg),
			"already reconciled -> no second reload"
		);
	}

	#[test]
	fn do_persist_skips_overwriting_a_kern_when_the_graph_is_stale() {
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::locks::{read_recovered, write_recovered};
		use crate::base::types::{mk_entity, EntityKind, Kern};

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};

		let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
		let root_id = read_recovered(&g).root.id.clone();

		let mut k = Kern::new("k", &root_id);
		k.entities.insert(
			"e".into(),
			mk_entity("e", "durable fact", 1.0, EntityKind::Claim),
		);
		commit_extra_kern_via_store(&g, k);

		write_recovered(&g)
			.kerns
			.insert("k".into(), Kern::new("k", &root_id));
		crate::tick::tasks::do_persist(&g, "k");

		// Read disk back through the same store handle.
		let on_disk = read_recovered(&g)
			.store()
			.unwrap()
			.load_one_kern("k")
			.unwrap()
			.expect("k still on disk");
		assert!(
			on_disk.entities.contains_key("e"),
			"stale per-kern persist was skipped — the CLI's entity survives"
		);
	}

	#[test]
	fn periodic_snapshot_closes_the_unflushed_mutation_crash_window() {
		// "Crash" = drop every handle with NO shutdown flush, then reopen the dir.
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::locks::{read_recovered, write_recovered};
		use crate::base::types::Kern;

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};

		{
			let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
			let root_id = read_recovered(&g).root.id.clone();
			write_recovered(&g).register(Kern::new("unflushed", &root_id));
		} // crash: all env handles dropped, no save
		{
			let g = super::load_graph(&cfg);
			assert!(
				g.loaded("unflushed").is_none(),
				"window proven: an unflushed mutation is lost across a crash"
			);
		}

		{
			let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
			let mut last = read_recovered(&g).mutation_epoch();
			assert!(
				!super::snapshot_if_dirty(&g, &cfg, &mut last),
				"clean graph -> the interval snapshot is a no-op"
			);
			let root_id = read_recovered(&g).root.id.clone();
			write_recovered(&g).register(Kern::new("snapshotted", &root_id));
			assert!(
				super::snapshot_if_dirty(&g, &cfg, &mut last),
				"mutation epoch moved -> the snapshot flushes"
			);
			assert!(
				!super::snapshot_if_dirty(&g, &cfg, &mut last),
				"no further mutation -> the next interval skips the rewrite"
			);
		} // crash again: no shutdown flush
		{
			let g = super::load_graph(&cfg);
			assert!(
				g.loaded("snapshotted").is_some(),
				"the snapshot bounded the loss window: the mutation survived the crash"
			);
		}
	}

	#[test]
	fn cluster_migrated_entities_survive_a_crash_after_the_spawn_persists() {
		// Guards the old data-loss window: Persist(parent) rewrote the parent row
		// without the migrated entities while the spawned child went unpersisted.
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::constants::KERN_MIN_CLUSTER_SIZE;
		use crate::base::locks::{read_recovered, write_recovered};
		use crate::base::types::{mk_entity, EntityKind, Kern};

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};
		let entity_ids: Vec<String> = (0..KERN_MIN_CLUSTER_SIZE)
			.map(|i| format!("spill{i}"))
			.collect();

		{
			let g = Arc::new(RwLock::new(super::load_graph(&cfg)));
			let root_id = read_recovered(&g).root.id.clone();
			let mut k = Kern::new("k", &root_id);
			k.anchor_text = "named".into();
			k.anchor_vec = vec![1.0, 0.0];
			for id in &entity_ids {
				let mut e = mk_entity(id, id, 1.0, EntityKind::Claim);
				e.vector = vec![0.0, 1.0];
				k.entities.insert(id.clone(), e);
			}
			write_recovered(&g).register(k);
			super::save_graph_guarded(&g, &cfg);

			crate::tick::tick_sync(&g, "k", None, None, None);
			let child_exists = {
				let gg = read_recovered(&g);
				let parent = gg.loaded("k").expect("parent kern still loaded");
				assert!(
					parent.entities.is_empty(),
					"precondition: the cluster migrated out of the parent"
				);
				!parent.children.is_empty()
			};
			assert!(child_exists, "precondition: a child kern was spawned");
		}

		let g = super::load_graph(&cfg);
		for id in &entity_ids {
			let found = g.all().iter().any(|k| k.entities.contains_key(id));
			assert!(
				found,
				"entity {id} must survive the crash — the spawned child's Persist landed it on disk"
			);
		}
	}

	#[test]
	fn apply_graph_config_spills_to_disk_when_threshold_enabled() {
		use crate::base::constants::KERN_CAP_DISABLED;
		use crate::base::graph::GraphGnn;
		use crate::base::types::{Entity, EntityStatus, Kern};
		use crate::base::vector_backend::VectorBackend;
		use crate::config::GraphConfig;

		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		let mut kern = Kern::new("k", "");
		for i in 0..30 {
			let v: Vec<f32> = (0..8)
				.map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin() as f32)
				.collect();
			kern.entities.insert(
				format!("e{i}"),
				Entity {
					id: format!("e{i}"),
					vector: v,
					status: EntityStatus::Active,
					..Default::default()
				},
			);
		}
		g.kerns.insert("k".into(), kern);
		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Resident(_)),
			"default load stays in-RAM"
		);

		let cfg = GraphConfig {
			max_kerns: KERN_CAP_DISABLED,
			max_ledger_entries: 10_000,
			disk_threshold: 10,
		};
		super::apply_graph_config(&mut g, &cfg);
		assert!(
			matches!(g.entity_idx, VectorBackend::Disk { .. }),
			"configured threshold spills at startup"
		);

		let mut g2 = GraphGnn::new();
		g2.data_dir = dir.path().to_string_lossy().into_owned();
		let cfg_off = GraphConfig {
			max_kerns: KERN_CAP_DISABLED,
			max_ledger_entries: 10_000,
			disk_threshold: KERN_CAP_DISABLED,
		};
		super::apply_graph_config(&mut g2, &cfg_off);
		assert!(
			matches!(g2.entity_idx, VectorBackend::Resident(_)),
			"default-off stays in-RAM"
		);
	}
}
