//! CLI command handlers for the `kern` binary: each subcommand (admin, graph
//! ops, ingest, MCP, profile, reembed, …) dispatched from `main`.

mod admin;
mod graph_ops;
mod ingest_cmd;
mod mcp_cmd;
mod profile_cmd;
mod query;
mod reembed;

/// Register kern MCP servers in the project's `.mcp.json`.
/// Called at mux and daemon startup — idempotent, safe to call every boot.
pub(crate) use mcp_cmd::ensure_mcp_registered;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::base::graph::GraphGnn;
use crate::base::locks::read_recovered;

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
	/// Time every hot path (load, embed, search, query stages, distill,
	/// digest) and print a scaled timeline. Read-only.
	Profile {
		/// Query text driving the embed/search/query stages.
		#[arg(long, default_value = "what is this project about")]
		text: String,
		/// Skip LLM-dependent stages (hyde, rerank, answer, distill).
		#[arg(long)]
		no_llm: bool,
	},
	/// Reap empty unnamed kerns, persist, and shrink data.mdb (daemon stopped).
	Gc,
	/// Shrink data.mdb to its live size without reaping (daemon stopped). LMDB
	/// keeps freed pages in-file after a delete; this rewrites the env compactly.
	Compact,
	/// Manage anchors: named top-level buckets the root routes memories into.
	/// Memories that match no anchor fall through to `generic`.
	Anchor {
		#[command(subcommand)]
		action: AnchorAction,
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
	/// One-shot: migrate a legacy file-shard data dir to the LMDB store (in place).
	/// The old `.kern` files are left for you to delete.
	Migrate {
		/// Data dir to migrate; defaults to the configured data_dir.
		path: Option<String>,
	},
	/// Run a timed self-improvement hunt (feature-gated).
	#[cfg(feature = "hunt")]
	Hunt {
		#[arg(long, default_value = "60")]
		secs: u64,
	},
	/// Start the long-lived daemon (same as `--daemon`). Convenience alias.
	Daemon,
}

#[derive(Subcommand)]
pub enum AnchorAction {
	/// Add a named anchor; `text` is embedded into its routing vector.
	Add {
		name: String,
		text: String,
		#[arg(long)]
		embed_url: Option<String>,
		#[arg(long)]
		embed_model: Option<String>,
	},
	/// List the root's anchors.
	List,
	/// Remove a named anchor; its memories fall back to generic.
	Remove { name: String },
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

/// Apply the `[graph]` config to a freshly loaded graph. `load_dir` already ran
/// `rebuild_index` once, but with the DEFAULT `disk_threshold` — so when spilling
/// is enabled we rebuild ONCE more here, after the configured threshold is set, or
/// the disk tier would stay inert until some later organic rebuild. Guarded so the
/// common default-off case pays nothing (no second rebuild).
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
			// load_dir only errors on a real LMDB/IO fault (an empty store yields a
			// fresh graph, not an error). Still bind a store so saves persist.
			let mut g = GraphGnn::new();
			g.data_dir = cfg.data_dir.clone();
			if let Ok(store) = crate::base::store::Store::open(&cfg.data_dir) {
				g.set_store(std::sync::Arc::new(store));
			}
			g
		}
	};
	apply_graph_config(&mut g, &cfg.graph);
	// Wire the configured BM25 parameters into the lexical index. Without this the
	// `config.retrieval.bm25_k1`/`bm25_b` fields were dead — the index always scored
	// with the hardcoded construction defaults regardless of config.
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

/// data.mdb size above which a startup self-heal (reap + compact) is worth its
/// double-load cost. A clean graph is a few MB; this only fires on the runaway
/// bloat (observed up to 4 GiB) the unnamed-kern fragmentation used to leave.
const SELF_HEAL_BLOAT_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

/// If `data.mdb` is bloated, reap empty kerns and compact the env in place — once,
/// at startup, before the serving env opens. MUST be called only while this
/// process holds the cwd exclusively (no other daemon serving it), or the reap
/// would race a live writer and the swap would fail on a held file. Idempotent and
/// best-effort: any failure is logged and startup continues with the un-compacted
/// (but still functional) store.
fn maybe_self_heal_store(cfg: &crate::config::Config) {
	let data = std::path::Path::new(&cfg.data_dir).join("data.mdb");
	let len = std::fs::metadata(&data).map(|m| m.len()).unwrap_or(0);
	if len < SELF_HEAL_BLOAT_BYTES {
		return;
	}
	tracing::info!(target: "kern.startup", bytes = len, "data.mdb is bloated; self-healing (reap + compact)");

	// Reap empties and persist the deletions in a throwaway graph, then drop it so
	// its env handle releases before the compaction swap.
	{
		let mut g = load_graph(cfg);
		let (before, reaped, after) = g.gc_empty_kerns_counted();
		if reaped > 0 {
			save_graph(&g);
			eprintln!("kern: self-heal reaped {reaped} empty kerns ({before} -> {after})");
		}
		// g (and its env handle) dropped here.
	}
	// `compact_dir` opens its own env and closes it deterministically before the
	// swap, so the just-reaped data.mdb shrinks to its live size.
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

/// Build the shared blocking embed closure used by the tick worker and the
/// `profile` command: clone the LLM client into an `EmbedFunc` that drives
/// `embed` on the current runtime via `block_on_in_place`. One definition so the
/// call sites can't drift. (The standalone MCP path uses a distinct
/// runtime-handle variant and is intentionally not routed through this.)
pub(crate) fn embed_fn(client: &Client) -> crate::types::EmbedFunc {
	let c = client.clone();
	std::sync::Arc::new(move |text: &str| -> Result<Vec<f64>, String> {
		let c = c.clone();
		let text = text.to_string();
		match crate::llm::block_on_in_place(c.embed(&text)) {
			Some(r) => r.map_err(|e| e.to_string()),
			None => Err("no runtime".to_string()),
		}
	})
}

/// Build the full reason+answer+embed [`Client`] shared by the long-lived daemon
/// (`run_server`) and the standalone MCP server (`mcp_cmd::run_standalone`). The
/// reason endpoint is passed in already resolved — `run_server` lets a CLI flag
/// win over the `[reason]` config section, the MCP path uses config directly —
/// while answer and embed are ALWAYS taken from config: the daemon must embed
/// with the same model the graph was built with, never a CLI default, or every
/// cosine degenerates on a dimension mismatch.
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
		Commands::Daemon => {
			// Fallback: `main.rs` handles this arm first (before calling dispatch),
			// but keeping it here ensures `dispatch` handles all Commands variants,
			// so future call sites don't need to special-case Daemon.
			let default_cli = Cli {
				command:     None,
				daemon:      true,
				mcp_addr:    String::new(),
				mcp_stdio:   false,
				embed_url:   crate::config::DEFAULT_EMBED_URL.to_string(),
				embed_model: crate::config::DEFAULT_EMBED_MODEL.to_string(),
				reason_url:  String::new(),
				reason_model: String::new(),
			};
			run_server(&default_cli, cfg).await;
		}
	}
}

/// Handles produced by [`bootstrap`]: the live engine plus the side-channels a
/// caller's serve/park loop (or TUI) needs.
pub(crate) struct EngineHandle {
	pub server:  std::sync::Arc<crate::mcp::Server>,
	pub graph:   SharedGraph,
	pub worker:  std::sync::Arc<crate::ingest::Worker>,
	pub task_q:  std::sync::Arc<crate::tick::queue::Queue>,
	pub llm:     crate::llm::Client,
}

/// Build the engine stack (graph + worker + tick + MCP server) and spawn every
/// background service: watchdog, keepalive, viewer, session-mirror, file-watcher,
/// capture, gossip, maintenance tick. Shared by `run_server` (headless daemon)
/// and `run_mux` (TUI host). Does NOT register `.mcp.json`, does NOT bind
/// `kern.sock`, and does NOT block — the caller owns the serve/park loop and
/// decides whether to attach a TUI. `mux` is threaded into `mcp::Server` so the
/// comms tools advertise + dispatch against the live pane registry.
pub(crate) async fn bootstrap(
	cli: &Cli,
	cfg: &crate::config::Config,
	mux: Option<std::sync::Arc<std::sync::Mutex<crate::mux::registry::PaneRegistry>>>,
) -> EngineHandle {
	if let Some(j) = journal::global() {
		j.set_max_bytes(cfg.journal.max_today_bytes);
	}

	// Self-heal a bloated store BEFORE the serving env opens. LMDB never returns
	// freed pages to the OS, so a graph that was once fragmented to a runaway of
	// empty unnamed kerns leaves a multi-GB `data.mdb` that every load must mmap
	// and that the startup reap alone can't shrink. Here — while we still hold the
	// cwd exclusively (the mux calls bootstrap only after winning kern.sock) and
	// no serving env is open yet — reap the empties and rewrite the file compactly.
	// Guarded by a size threshold so a normal small graph pays nothing.
	maybe_self_heal_store(cfg);

	// Runtime watchdog. A wedged daemon — deadlock on the graph lock, a panic
	// loop, or every worker thread pinned in a blocking LLM call — keeps holding
	// the hub TCP socket, so no peer can bind it and the viewer stays dead until
	// the process is killed by hand (observed: a daemon stopped serving `/graph`
	// AND heartbeating yet held `:7700` for 8+ minutes). An async task bumps
	// `beat` every second; a DEDICATED OS thread — immune to runtime starvation,
	// unlike any tokio task — force-exits the process if `beat` stops advancing.
	// Exiting frees the socket so a healthy peer takes over the hub within
	// `viewer::FAILOVER_RETRY`. The threshold (30s) is far above any single LLM
	// call: normal blocking work pins one worker, never the time driver, so the
	// beat keeps advancing — only a TOTAL stall trips the watchdog.
	spawn_watchdog();
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
	let llm_client = server_llm_client(cfg, &reason_url, &reason_model);

	let llm_fn: Option<crate::ingest::LlmFunc> = if !reason_url.is_empty() {
		Some(Arc::new(llm_client.complete_func()))
	} else {
		None
	};

	// Keep the embedding AND answer models resident. Ollama unloads after ~5 min
	// idle, and the OpenAI-compat /v1 endpoint kern uses ignores `keep_alive`, so
	// the next call pays a multi-second cold reload (~7s for qwen3-embedding, more
	// for the 4b answer model). A tiny embed + a 1-token answer ping every 4 min
	// re-touches both so retrieval and the user-facing /ask stay warm. Cheap and
	// self-contained — no dependency on OLLAMA_KEEP_ALIVE.
	spawn_keepalive(&llm_client);

	let tick_llm: crate::tick::tasks::LlmFunc = Arc::new(llm_client.complete_func());
	let tick_embed: crate::tick::tasks::EmbedFunc = embed_fn(&llm_client);

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

	// Self-heal the unnamed-kern fragmentation on startup. The historical spawn
	// runaway (now fixed) left graphs with tens of thousands of empty kerns
	// persisted to disk; since retrieval, tick, and the viewer are all O(loaded
	// kerns), that bloat taxes every request. Reap them once here so every
	// restart converges to a clean graph, then persist the compacted form.
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
			save_graph(&read_recovered(&g));
		}
	}

	// Build the MCP server early — shared by the viewer (tool endpoints) and
	// the RPC surface below. Created here so both can reference the same Arc.
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
		mux,
	});

	spawn_viewer(cfg, &g, &llm_client, &q, &mcp_server);

	spawn_session_mirror(cfg, &worker);
	spawn_compactor(cfg, &g, std::sync::Arc::new(llm_client.complete_func()));

	spawn_file_watcher(cfg, &worker);

	spawn_capture(cfg, &worker, &llm_fn, &g);

	start_gossip(cfg, &g, &q, &save_fn).await;

	spawn_maintenance_tick(cfg, &g, &q);

	EngineHandle { server: mcp_server, graph: g, worker, task_q: q, llm: llm_client }
}

pub async fn run_server(cli: &Cli, cfg: &crate::config::Config) {
	// Register kern in .claude/settings.json so Claude Code picks it up.
	{
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		ensure_mcp_registered(&cwd);
	}

	let h = bootstrap(cli, cfg, None).await;
	let g = h.graph.clone();
	let worker = h.worker.clone();
	let q = h.task_q.clone();
	let llm_client = h.llm.clone();
	let mcp_server = h.server.clone();

	let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
	tokio::spawn(async move {
		tokio::signal::ctrl_c().await.ok();
		let _ = shutdown_tx.send(());
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
					tracing::error!(target: "kern.mcp_sse", error = %e, "MCP-over-HTTP server exited");
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

/// Shared shape of the live graph handle the daemon subsystems read/write.
type SharedGraph = Arc<std::sync::RwLock<GraphGnn>>;

/// Dedicated OS-thread watchdog: force-exits the process if the async runtime
/// stalls (graph deadlock or total worker starvation) so a healthy peer can take
/// over the hub socket. An async task bumps `beat` every second; the OS thread —
/// immune to runtime starvation — exits if `beat` stops advancing for ~30s.
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
						// stderr → daemon.err.log, so the next wedge is visible.
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

/// Keep the embedding AND answer models resident. Ollama unloads after ~5 min
/// idle and the /v1 endpoint ignores `keep_alive`, so the next call pays a cold
/// reload. A tiny embed + a 1-token answer ping every 4 min re-touches both
/// (warmed concurrently — they live in separate runners) so retrieval and `/ask`
/// stay warm. The first tick fires immediately, loading both at boot.
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

/// Live graph viewer — a read-only web UI over the current graph. Localhost by
/// default (`cfg.serve.viewer`); empty disables it.
fn spawn_viewer(
	cfg: &crate::config::Config,
	g: &SharedGraph,
	llm_client: &Client,
	q: &Arc<crate::tick::queue::Queue>,
	mcp_server: &Arc<crate::mcp::Server>,
) {
	if cfg.serve.viewer.is_empty() {
		return;
	}
	let vg = g.clone();
	let vaddr = cfg.serve.viewer.clone();
	let viewer_llm = llm_client.clone();
	let viewer_retrieval = cfg.retrieval.clone();
	let viewer_q = q.clone();
	let viewer_mcp = mcp_server.clone();
	tokio::spawn(async move {
		if let Err(e) = crate::viewer::run(vg, viewer_llm, viewer_retrieval, viewer_q, viewer_mcp, &vaddr).await {
			tracing::warn!(target: "kern.viewer", error = %e, "graph viewer failed to start");
		}
	});
}

/// Slice K — session mirror. Tails the shared journal `fork_*` lifecycle events
/// and ingests each new fork as a `Document` entity with `Source::Session`.
/// The live mirror tails `today.jsonl`; the SQLite archive is only pruned here
/// (a missing history.db does not disable mirroring).
fn spawn_session_mirror(cfg: &crate::config::Config, worker: &Arc<crate::ingest::Worker>) {
	use crate::ingest::session_mirror::{run, SessionMirror, WorkerSink};
	let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

	// Archive maintenance (independent of live mirroring): prune the SQLite
	// history of days older than the retention window. Best-effort — a missing
	// history.db must NOT disable the live mirror, which reads today.jsonl.
	if cfg.journal.retain_days > 0 {
		match journal::History::open(&cwd) {
			Ok(history) => match history.retain_days(cfg.journal.retain_days) {
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
			},
			Err(e) => tracing::warn!(
				target: "kern.journal",
				error = %e,
				"history.db open failed; archive prune skipped"
			),
		}
	}

	// Live mirror: tail today.jsonl for fork lifecycle events. This is the file
	// `journal::emit` writes, so mux panes are mirrored as they open.
	let sink = WorkerSink::new(worker.clone());
	let mut sm = SessionMirror::new(sink);
	sm.set_max_seen(cfg.ingest.session_mirror_max_seen);
	let mirror = Arc::new(tokio::sync::Mutex::new(sm));
	let journal_path = cwd.join(".kern").join("journal").join("today.jsonl");
	tokio::spawn(run(journal_path, mirror, std::time::Duration::from_secs(2)));
}

/// Out-of-band journal compactor. Drains the dated segments produced by
/// `DayJournal` rollover into `history.db` (the machine-queryable archive),
/// deleting each segment after a successful insert. The optional Obsidian
/// "memory of the day" digest is rendered here too when enabled.
fn spawn_compactor(cfg: &crate::config::Config, graph: &SharedGraph, llm: crate::types::LlmFunc) {
	use crate::ingest::compactor::run;
	let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
	let interval = std::time::Duration::from_secs(cfg.journal.compactor_interval_secs.max(1));
	let export = cfg.journal.obsidian_export;
	let vault = cfg.journal.obsidian_vault.clone();
	tokio::spawn(run(cwd, interval, export, vault, graph.clone(), llm));
}

/// Slice O — kern-side filesystem watcher. Off unless `[watcher] enabled = true`.
/// Roots default to cwd when enabled but none are listed.
fn spawn_file_watcher(cfg: &crate::config::Config, worker: &Arc<crate::ingest::Worker>) {
	if !cfg.watcher.enabled {
		return;
	}
	use crate::ingest::file_watcher::{run as run_file_watcher, KernFileWatcherSink};
	use watcher::IgnoreRules;
	// `effective_roots` applies the documented "empty roots → cwd" rule.
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

/// Claude-Code memory: capture spool drain + recall digest writer. Both
/// file-mediated; on by default, disable via `[capture] enabled = false`.
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

	// Capture drain: spool deltas -> distill -> enqueue -> archive.
	if let Some(llm_fn) = llm_fn.clone() {
		let spool = cwd.join(&cfg.capture.dir);
		let worker_c = worker.clone();
		let dedup = cfg.ingest.dedup_threshold;
		let poll = std::time::Duration::from_secs(cfg.capture.poll_secs);
		let done_retention = std::time::Duration::from_secs(cfg.capture.done_retention_secs);
		tokio::spawn(crate::ingest::capture_spool::run(
			spool, worker_c, llm_fn, dedup, poll, done_retention,
		));
	} else {
		tracing::warn!(
			target: "kern.capture",
			"capture: spool drain inactive — add a [reason] section to kern.toml to enable distillation; deltas will accumulate in .kern/capture/ and will be processed once the daemon restarts with a reason LLM configured"
		);
	}

	// Digest writer: periodically snapshot purpose + hot thoughts.
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

/// Federation: start the gossip node so this kern can share/receive knowledge
/// with peers. OFF by default (`[gossip] enabled`). When on, binds a TCP
/// listener, runs heartbeat, and (optionally) LAN multicast discovery.
async fn start_gossip(
	cfg: &crate::config::Config,
	g: &SharedGraph,
	q: &Arc<crate::tick::queue::Queue>,
	save_fn: &Arc<dyn Fn() + Send + Sync>,
) {
	if !cfg.gossip.enabled {
		return;
	}
	let network_id = {
		let g = read_recovered(g);
		g.network_id.clone()
	};
	let node = crate::gossip::node::Node::new(&cfg.gossip.addr, &network_id, cfg.gossip.peers.clone());
	// Wire the configured ledger cap. Without this `config.graph.max_ledger_entries`
	// was dead — the routing/thought ledger always used the hardcoded
	// DEFAULT_LEDGER_CAP regardless of config.
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

/// Autonomous maintenance tick: pulses the root (heat decay + stigmergy GC of
/// cold nodes) and re-enqueues clustering on a timer so an idle daemon still
/// decays, merges, and evicts. `interval_secs = 0` disables it.
fn spawn_maintenance_tick(cfg: &crate::config::Config, g: &SharedGraph, q: &Arc<crate::tick::queue::Queue>) {
	if cfg.tick.interval_secs == 0 {
		return;
	}
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

#[cfg(test)]
mod entry_point_tests {
	use super::Commands;

	#[test]
	fn daemon_subcommand_exists() {
		// Regression guard: confirms Commands::Daemon compiles.
		let _ = Commands::Daemon;
	}

	#[test]
	fn apply_graph_config_spills_to_disk_when_threshold_enabled() {
		// Regression: load_dir rebuilds with the DEFAULT threshold; apply_graph_config
		// must rebuild again once the configured threshold is set, or a configured
		// disk_threshold never engages at startup (the entity index stays in-RAM).
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
			let v: Vec<f64> = (0..8).map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin()).collect();
			kern.entities.insert(
				format!("e{i}"),
				Entity { id: format!("e{i}"), vector: v, status: EntityStatus::Active, ..Default::default() },
			);
		}
		g.kerns.insert("k".into(), kern);
		g.rebuild_index(); // mimics load_dir's rebuild under the default threshold
		assert!(matches!(g.entity_idx, VectorBackend::Resident(_)), "default load stays in-RAM");

		// Enabled threshold -> apply_graph_config must rebuild and spill.
		let cfg = GraphConfig { max_kerns: KERN_CAP_DISABLED, max_ledger_entries: 10_000, disk_threshold: 10 };
		super::apply_graph_config(&mut g, &cfg);
		assert!(matches!(g.entity_idx, VectorBackend::Disk { .. }), "configured threshold spills at startup");

		// Disabled threshold -> no spill, no redundant rebuild side effects.
		let mut g2 = GraphGnn::new();
		g2.data_dir = dir.path().to_string_lossy().into_owned();
		let cfg_off = GraphConfig { max_kerns: KERN_CAP_DISABLED, max_ledger_entries: 10_000, disk_threshold: KERN_CAP_DISABLED };
		super::apply_graph_config(&mut g2, &cfg_off);
		assert!(matches!(g2.entity_idx, VectorBackend::Resident(_)), "default-off stays in-RAM");
	}
}
