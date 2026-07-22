use trnsprt::kern_rpc::{AuthReq, PRINCIPAL_CLI};
use trnsprt::typed::Endpoint;

use crate::base::util::short_id;

use super::route::{route_to, Routed};
use super::{
	load_graph, save_graph_unguarded, with_graph, ClaimKindAction, Client, GravitonAction,
	UnnamedAction,
};

pub(super) fn cmd_compress(src: &str, mode_str: &str, out: Option<&str>) {
	let Some(mode) = crate::quant::QuantizationMode::parse(mode_str) else {
		eprintln!("compress: unknown mode '{mode_str}' (expected: none | int8)");
		return;
	};
	let mode_label = mode.as_str();
	let out_dir = out
		.map(|s| s.to_string())
		.unwrap_or_else(|| format!("{src}.{mode_label}"));
	if std::path::Path::new(&out_dir).exists() {
		eprintln!("compress: output path '{out_dir}' already exists; refusing to overwrite");
		return;
	}
	match crate::base::persist::compress_dir(src, &out_dir, mode) {
		Ok(()) => {
			let bpd = mode.bytes_per_dim();
			println!(
				"compressed {src} -> {out_dir}  mode={} (~{:.1} bytes/dim)",
				mode.as_str(),
				bpd,
			);
		}
		Err(e) => eprintln!("compress: {e}"),
	}
}

pub(super) async fn cmd_health(cfg: &crate::config::Config) {
	let g = load_graph(cfg);
	let h = crate::base::health::graph_health_stats(&g);

	println!("data_dir:    {}", g.data_dir);
	if h.gravitons.is_empty() {
		println!("gravitons:     (none)");
	} else {
		println!("gravitons:     {}", h.gravitons.join(", "));
	}
	println!("kerns:       {}", h.kerns);
	println!("thoughts:    {} (unnamed: {})", h.entities, h.unnamed);
	println!("reasons:     {}", h.reasons);
	println!("claim kinds: {}", g.root.claim_kinds.len());
	println!(
		"embed:       {} (dim {}){}",
		if h.embed_model.is_empty() {
			"(unstamped)"
		} else {
			&h.embed_model
		},
		h.embed_dim,
		if h.embed_mismatch {
			"  MISMATCH: the index was built with a different model"
		} else {
			""
		},
	);
	println!("evicted:     {} cold rows dropped", h.cold_evicted);
	// Fail-open is the policy; invisible fail-open is the defect (ROADMAP item 7).
	// Print the line only when something actually degraded, so a healthy kern stays
	// quiet and a nonzero count is impossible to scroll past.
	let degraded = h.query_dim_rejected
		+ h.below_floor_deliveries
		+ h.clock_skew_skips
		+ h.ingest_dropped_chunks
		+ h.remote_cap_dropped
		+ h.unspilled_drops
		+ h.ingest_queue_refused;
	if degraded > 0 {
		println!(
			"degraded:    {} off-model queries dropped, {} below-floor deliveries, {} clock-skewed entities GC could not age, {} chunks lost to embedding, {} remote ids refused at the cap, {} dropped with nowhere to spill, {} ingest jobs refused at the queue bound",
			h.query_dim_rejected,
			h.below_floor_deliveries,
			h.clock_skew_skips,
			h.ingest_dropped_chunks,
			h.remote_cap_dropped,
			h.unspilled_drops,
			h.ingest_queue_refused
		);
	}
	for line in tick_health_lines(daemon_health(cfg).await.as_ref()) {
		println!("{line}");
	}

	for k in g.all() {
		let label = if k.graviton_text.is_empty() {
			"[unnamed]"
		} else {
			&k.graviton_text
		};
		println!(
			"  kern:{}  thoughts:{}  reasons:{}",
			label,
			k.entities.len(),
			k.reasons.len(),
		);
	}
}

// The tick queue lives in the daemon; an offline CLI has no view of it. One
// attempt, no retry: `kern health` must not stall when nothing is serving.
async fn daemon_health(cfg: &crate::config::Config) -> Option<trnsprt::kern_rpc::HealthRes> {
	use trnsprt::kern_rpc::{KernRpcClient, PRINCIPAL_CLI};
	use trnsprt::typed::{Endpoint, JsonEnvelopeCodec};

	let client = KernRpcClient::<JsonEnvelopeCodec>::connect_endpoint_with_retry(
		&Endpoint::kern(),
		&crate::rpc::caller_of(cfg, PRINCIPAL_CLI),
		1,
		std::time::Duration::ZERO,
	)
	.await
	.ok()?;
	client.health().await.ok().filter(|h| h.ok)
}

fn tick_health_lines(h: Option<&trnsprt::kern_rpc::HealthRes>) -> Vec<String> {
	let Some(h) = h else {
		return vec!["tick:        (no daemon serving this directory)".to_string()];
	};
	let mut lines = vec![
		format!(
			"tick:        queue {} | done {} | avg {} ms",
			h.queue_depth, h.tasks_done, h.task_avg_ms
		),
		format!(
			"degraded:    {} panics | {} failures | {} refused GNN trainings",
			h.task_panics, h.task_failures, h.gnn_train_refused
		),
	];
	if !h.last_task_panic.is_empty() {
		lines.push(format!("  last panic:   {}", h.last_task_panic));
	}
	if !h.last_task_failure.is_empty() {
		lines.push(format!("  last failure: {}", h.last_task_failure));
	}
	lines
}

// Daemon must be stopped: a live daemon would race and re-persist the bloated graph.
pub(super) fn cmd_gc(cfg: &crate::config::Config) {
	let _lock = match crate::base::lock::acquire(&cfg.data_dir, "gc") {
		Ok(l) => l,
		Err(e) => {
			eprintln!("gc: {e}");
			eprintln!("  stop it first — a live daemon re-persists the graph this reaped from");
			return;
		}
	};
	let mut g = load_graph(cfg);
	let (before, reaped, after) = g.gc_empty_kerns_counted();
	save_graph_unguarded(&g);
	println!("gc: reaped {reaped} empty kerns ({before} -> {after})");

	// Drop the graph FIRST to release its env handle: compact_dir closes its own
	// env deterministically — a lazy drop on Windows leaves data.mdb mmap'd.
	drop(g);
	match crate::base::store::compact_dir(&cfg.data_dir) {
		Ok((old, new)) => println!(
			"gc: compacted data.mdb {} -> {} ({:.0}% reclaimed)",
			human_bytes(old),
			human_bytes(new),
			if old > new && old > 0 {
				(old - new) as f64 * 100.0 / old as f64
			} else {
				0.0
			},
		),
		Err(e) => eprintln!("gc: compaction failed: {e}"),
	}
}

// Daemon must be stopped: compaction swaps data.mdb underneath any open env.
pub(super) fn cmd_compact(cfg: &crate::config::Config) {
	let _lock = match crate::base::lock::acquire(&cfg.data_dir, "compact") {
		Ok(l) => l,
		Err(e) => {
			eprintln!("compact: {e}");
			eprintln!("  stop it first — compaction renames data.mdb under any open environment");
			return;
		}
	};
	match crate::base::store::compact_dir(&cfg.data_dir) {
		Ok((old, new)) => println!(
			"compact: data.mdb {} -> {} ({:.0}% reclaimed)",
			human_bytes(old),
			human_bytes(new),
			if old > 0 {
				(old - new) as f64 * 100.0 / old as f64
			} else {
				0.0
			},
		),
		Err(e) => eprintln!("compact: failed: {e}"),
	}
}

fn human_bytes(n: u64) -> String {
	const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
	let mut v = n as f64;
	let mut i = 0;
	while v >= 1024.0 && i < U.len() - 1 {
		v /= 1024.0;
		i += 1;
	}
	if i == 0 {
		format!("{n} B")
	} else {
		format!("{v:.1} {}", U[i])
	}
}

fn print_graviton_added(name: &str, mass: f64) {
	println!("graviton added: {name} (mass {mass})");
}

fn print_graviton_removed(name: &str) {
	println!("graviton removed: {name}");
}

pub(super) async fn cmd_graviton(cfg: &crate::config::Config, action: GravitonAction) {
	graviton_at(
		cfg,
		&Endpoint::kern(),
		&crate::rpc::caller_of(cfg, PRINCIPAL_CLI),
		action,
	)
	.await
}

// Routed first for the same reason as forget: `with_graph` writes the whole kern
// map back unguarded, so a local graviton edit beside a serving daemon drops
// everything that daemon has committed since this process loaded.
async fn graviton_at(
	cfg: &crate::config::Config,
	endpoint: &Endpoint,
	auth: &AuthReq,
	action: GravitonAction,
) {
	match action {
		GravitonAction::Add {
			name,
			text,
			mass,
			embed,
		} => {
			let mass = mass.unwrap_or(1.0);
			// Routed before the embed: the daemon owns the vector it stores, and
			// embedding here would be a second call to the same model for nothing.
			match route_to(
				endpoint,
				auth,
				"graviton",
				serde_json::json!({"action": "add", "name": &name, "text": &text, "mass": mass}),
			)
			.await
			{
				Routed::Done(_) => return print_graviton_added(&name, mass),
				Routed::Refused(e) => return eprintln!("{e}"),
				Routed::NoDaemon => {}
			}
			let (url, model) = embed.resolve(cfg);
			let llm_client = Client::new_embed_only(url, model, &cfg.embed.key);
			// Multi-line seed = example statements, embedded separately and
			// mean-pooled (see accept::seed_examples for the measurement).
			let mut vecs = Vec::new();
			for ex in crate::base::accept::seed_examples(&text) {
				match llm_client.embed(&ex).await {
					Ok(v) => vecs.push(v),
					Err(e) => {
						eprintln!("embed: {e}");
						return;
					}
				}
			}
			let Some(vec) = crate::base::accept::mean_pool(&vecs) else {
				eprintln!("embed: empty or mismatched embeddings");
				return;
			};
			with_graph(cfg, |g| {
				crate::base::accept::add_graviton_with_mass(g, &name, vec, mass)
			});
			print_graviton_added(&name, mass);
		}
		GravitonAction::List => {
			let g = load_graph(cfg);
			println!("gravitons:");
			for r in graviton_rows(&g) {
				println!(
					"  {}  mass:{}  thoughts:{}  reasons:{}",
					r.name, r.mass, r.thoughts, r.reasons,
				);
			}
		}
		GravitonAction::Remove { name } => {
			match route_to(
				endpoint,
				auth,
				"graviton",
				serde_json::json!({"action": "remove", "name": &name}),
			)
			.await
			{
				Routed::Done(_) => return print_graviton_removed(&name),
				Routed::Refused(e) => return eprintln!("{e}"),
				Routed::NoDaemon => {}
			}
			let removed = with_graph(cfg, |g| crate::base::accept::remove_graviton(g, &name));
			if removed {
				print_graviton_removed(&name);
			} else {
				eprintln!("graviton not found: {name}");
			}
		}
	}
}

pub(crate) struct GravitonRow {
	pub(crate) name: String,
	pub(crate) mass: f64,
	pub(crate) thoughts: usize,
	pub(crate) reasons: usize,
}

pub(crate) fn graviton_rows(g: &crate::base::graph::GraphGnn) -> Vec<GravitonRow> {
	crate::base::accept::root_graviton_ids(g)
		.iter()
		.filter_map(|cid| g.loaded(cid))
		.map(|c| GravitonRow {
			name: c.graviton_text.clone(),
			mass: c.mass,
			thoughts: c.entities.len(),
			reasons: c.reasons.len(),
		})
		.collect()
}

fn print_claim_kind_added(name: &str) {
	println!("claim kind added: {name}");
}

fn print_claim_kind_removed(name: &str) {
	println!("claim kind removed: {name}");
}

pub(super) async fn cmd_claim_kind(cfg: &crate::config::Config, action: ClaimKindAction) {
	claim_kind_at(
		cfg,
		&Endpoint::kern(),
		&crate::rpc::caller_of(cfg, PRINCIPAL_CLI),
		action,
	)
	.await
}

async fn claim_kind_at(
	cfg: &crate::config::Config,
	endpoint: &Endpoint,
	auth: &AuthReq,
	action: ClaimKindAction,
) {
	match action {
		ClaimKindAction::Add { name, description } => {
			match route_to(
				endpoint,
				auth,
				"claim_kind",
				serde_json::json!({"action": "add", "name": &name, "description": &description}),
			)
			.await
			{
				Routed::Done(_) => return print_claim_kind_added(&name),
				Routed::Refused(e) => return eprintln!("{e}"),
				Routed::NoDaemon => {}
			}
			with_graph(cfg, |g| {
				g.root.claim_kinds.insert(name.clone(), description);
			});
			print_claim_kind_added(&name);
		}
		ClaimKindAction::Rm { name } => {
			match route_to(
				endpoint,
				auth,
				"claim_kind",
				serde_json::json!({"action": "rm", "name": &name}),
			)
			.await
			{
				Routed::Done(_) => return print_claim_kind_removed(&name),
				Routed::Refused(e) => return eprintln!("{e}"),
				Routed::NoDaemon => {}
			}
			with_graph(cfg, |g| {
				g.root.claim_kinds.remove(&name);
			});
			print_claim_kind_removed(&name);
		}
	}
}

pub(super) fn cmd_peers(cfg: &crate::config::Config) {
	print!("{}", peers_summary(cfg));
}

fn peers_summary(cfg: &crate::config::Config) -> String {
	let g = &cfg.gossip;
	let mut out = String::new();
	if !g.enabled {
		out.push_str("gossip:  disabled\n");
		out.push_str("  enable with [gossip] enabled = true in kern.toml\n");
		return out;
	}
	out.push_str("gossip:     enabled\n");
	out.push_str(&format!("addr:       {}\n", g.addr));
	out.push_str(&format!(
		"discovery:  {} (udp :{})\n",
		if g.discovery { "on" } else { "off" },
		g.discovery_port
	));
	if g.peers.is_empty() {
		out.push_str("peers:      (none configured)\n");
	} else {
		out.push_str(&format!("peers ({}):\n", g.peers.len()));
		for p in &g.peers {
			out.push_str(&format!("  {p}\n"));
		}
	}
	out.push_str("  (runtime-discovered peers visible in daemon logs)\n");
	out
}

#[cfg(test)]
mod peers_tests {
	use super::*;
	use crate::config::Config;

	#[test]
	fn peers_summary_gossip_disabled() {
		let cfg = Config::default();
		let s = peers_summary(&cfg);
		assert!(s.contains("disabled"), "disabled state shown");
		assert!(s.contains("enabled = true"), "enable hint shown");
	}

	#[test]
	fn peers_summary_enabled_no_seed_peers() {
		let mut cfg = Config::default();
		cfg.gossip.enabled = true;
		let s = peers_summary(&cfg);
		assert!(s.contains("enabled"), "enabled state shown");
		assert!(s.contains("none configured"), "empty peer list shown");
	}

	#[test]
	fn peers_summary_enabled_with_seed_peers() {
		let mut cfg = Config::default();
		cfg.gossip.enabled = true;
		cfg.gossip.peers = vec!["192.168.1.10:7400".into(), "192.168.1.11:7400".into()];
		let s = peers_summary(&cfg);
		assert!(s.contains("192.168.1.10:7400"), "first peer listed");
		assert!(s.contains("192.168.1.11:7400"), "second peer listed");
		assert!(s.contains("peers (2)"), "count shown");
	}
}

#[cfg(test)]
mod cmd_tests {
	use super::*;
	use crate::config::Config;

	fn temp_cfg() -> (tempfile::TempDir, Config) {
		let dir = tempfile::tempdir().expect("tempdir");
		let cfg = Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};
		(dir, cfg)
	}

	#[cfg(unix)]
	#[tokio::test(flavor = "multi_thread")]
	async fn claim_kind_add_then_remove_persists_through_the_graph() {
		let (_dir, cfg) = temp_cfg();
		// An endpoint nothing ever bound: the NoDaemon fallback, pinned so the
		// test can never reach a daemon the developer happens to be running.
		let ep = crate::test_support::scratch_endpoint("claim-kind-local");
		// A custom key, not a default: default keys re-inject on every load, so Rm
		// would appear to fail on the next load.
		let key = "custom_test_kind";

		claim_kind_at(
			&cfg,
			&ep,
			&crate::test_support::test_caller(),
			ClaimKindAction::Add {
				name: key.into(),
				description: "a custom kind".into(),
			},
		)
		.await;
		let g = load_graph(&cfg);
		assert_eq!(
			g.root.claim_kinds.get(key).map(String::as_str),
			Some("a custom kind"),
			"Add persists the claim kind onto the root",
		);

		claim_kind_at(
			&cfg,
			&ep,
			&crate::test_support::test_caller(),
			ClaimKindAction::Rm { name: key.into() },
		)
		.await;
		let g = load_graph(&cfg);
		assert!(
			!g.root.claim_kinds.contains_key(key),
			"Rm removes the custom claim kind"
		);
	}

	// The half of item 9 this closes: beside a serving daemon the command must
	// hand the write over, because the local path is `with_graph` — load, mutate,
	// `save_graph_unguarded` — which writes the whole kern map back with no epoch
	// check and drops every commit the daemon made since that load.
	#[cfg(unix)]
	#[tokio::test(flavor = "multi_thread")]
	async fn a_routed_claim_kind_add_lands_in_the_daemon_and_never_touches_the_store() {
		let (_dir, cfg) = temp_cfg();
		let ep = crate::test_support::scratch_endpoint("claim-kind-routed");
		let srv = crate::test_support::mcp_server();
		let graph = srv.graph.clone();
		crate::test_support::serving(srv, &ep).await;

		claim_kind_at(
			&cfg,
			&ep,
			&crate::test_support::test_caller(),
			ClaimKindAction::Add {
				name: "custom_test_kind".into(),
				description: "a custom kind".into(),
			},
		)
		.await;

		assert_eq!(
			graph
				.read()
				.root
				.claim_kinds
				.get("custom_test_kind")
				.map(String::as_str),
			Some("a custom kind"),
			"the serving daemon's own graph took the write"
		);
		assert!(
			!load_graph(&cfg)
				.root
				.claim_kinds
				.contains_key("custom_test_kind"),
			"the CLI's store was never written behind the daemon's back"
		);
	}

	#[cfg(unix)]
	#[tokio::test(flavor = "multi_thread")]
	async fn a_routed_graviton_remove_lands_in_the_daemon_and_never_touches_the_store() {
		let (_dir, cfg) = temp_cfg();
		// The local store carries the same graviton, so a command that fell through
		// to `with_graph` would visibly delete it here.
		with_graph(&cfg, |g| {
			crate::base::accept::add_graviton_with_mass(g, "docs", vec![1.0, 0.0], 1.0)
		});

		let ep = crate::test_support::scratch_endpoint("graviton-routed");
		let srv = crate::test_support::mcp_server();
		let graph = srv.graph.clone();
		crate::base::accept::add_graviton_with_mass(&mut graph.write(), "docs", vec![1.0, 0.0], 1.0);
		crate::test_support::serving(srv, &ep).await;

		graviton_at(
			&cfg,
			&ep,
			&crate::test_support::test_caller(),
			GravitonAction::Remove {
				name: "docs".into(),
			},
		)
		.await;

		assert!(
			graviton_rows(&graph.read()).is_empty(),
			"the serving daemon's own graph lost the graviton"
		);
		assert_eq!(
			graviton_rows(&load_graph(&cfg))
				.iter()
				.map(|r| r.name.clone())
				.collect::<Vec<_>>(),
			vec!["docs".to_string()],
			"the CLI's store is untouched — the daemon owns the write"
		);
	}

	#[tokio::test]
	async fn cmd_health_runs_on_a_fresh_graph_without_panicking() {
		let (_dir, cfg) = temp_cfg();
		cmd_health(&cfg).await;
	}

	#[test]
	fn tick_health_lines_report_both_degradation_counters() {
		let offline = tick_health_lines(None);
		assert_eq!(offline.len(), 1, "no daemon -> no invented numbers");
		assert!(offline[0].contains("no daemon"), "{offline:?}");

		let live = tick_health_lines(Some(&trnsprt::kern_rpc::HealthRes {
			ok: true,
			task_panics: 2,
			last_task_panic: "GnnPropagate[k]: boom".into(),
			task_failures: 3,
			last_task_failure: "GnnPropagate[k]: train epoch 0 forward".into(),
			..Default::default()
		}))
		.join("\n");
		assert!(live.contains("2 panics | 3 failures"), "{live}");
		assert!(
			live.contains("last panic:   GnnPropagate[k]: boom"),
			"{live}"
		);
		assert!(
			live.contains("last failure: GnnPropagate[k]: train epoch 0 forward"),
			"{live}"
		);
	}

	// This counter alone, every other one zero — which is the only state it is
	// ever seen in, since the trainer refusing has nothing to do with a task
	// panicking. A line gated on some other counter reports nothing here.
	#[test]
	fn a_refused_gnn_training_shows_with_no_other_counter_moving() {
		let lines = tick_health_lines(Some(&trnsprt::kern_rpc::HealthRes {
			ok: true,
			gnn_train_refused: 4,
			..Default::default()
		}));
		assert_eq!(lines.len(), 2, "counts only, no fault lines: {lines:?}");
		assert!(lines[1].contains("4 refused GNN trainings"), "{lines:?}");
	}

	#[test]
	fn a_clean_daemon_prints_no_last_fault_lines() {
		let lines = tick_health_lines(Some(&trnsprt::kern_rpc::HealthRes {
			ok: true,
			..Default::default()
		}));
		assert_eq!(
			lines.len(),
			2,
			"healthy tick reports counts only: {lines:?}"
		);
	}
}

pub(super) fn cmd_register(cfg: &crate::config::Config, path: &str) {
	// The loaded graph is bound to the SOURCE store, so write into a freshly
	// opened destination store — save_graph_unguarded would write back to the source.
	match crate::base::persist::load_dir(path) {
		Ok(g) => match crate::base::store::Store::open(&cfg.data_dir) {
			Ok(dest) => {
				let _ = crate::base::persist::save_graph_into(&dest, &g);
				println!("registered {path}");
			}
			Err(e) => eprintln!("register: {e}"),
		},
		Err(e) => eprintln!("load: {e}"),
	}
}

pub(super) fn cmd_unnamed(cfg: &crate::config::Config, action: UnnamedAction) {
	match action {
		UnnamedAction::List => {
			let g = load_graph(cfg);
			let mut found = false;
			for k in g.all() {
				if k.is_unnamed() {
					println!(
						"unnamed  id:{}  thoughts:{}",
						short_id(&k.id),
						k.entities.len()
					);
					found = true;
				}
			}
			if !found {
				println!("no unnamed kerns");
			}
		}
	}
}

fn default_root() -> String {
	let cwd = std::env::current_dir().unwrap_or_default();
	crate::config::Config::resolve_root(&cwd)
		.display()
		.to_string()
}

pub(super) async fn cmd_hub(action: Option<super::HubAction>, idle_unload_secs: u64) {
	use trnsprt::hub_rpc::{HubRpcClient, ResolveReq, UnloadReq};
	use trnsprt::typed::JsonEnvelopeCodec;

	match action {
		None => crate::hub::run_hub(idle_unload_secs).await,
		Some(super::HubAction::Resolve { root }) => {
			let root = root.unwrap_or_else(default_root);
			let client = match HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
				Ok(c) => c,
				Err(e) => {
					eprintln!("hub: not running ({e})");
					return;
				}
			};
			match client.resolve(ResolveReq { root: root.clone() }).await {
				Ok(res) if res.ok => println!(
					"{}  {}",
					if res.spawned { "spawned" } else { "running" },
					res.endpoint
				),
				Ok(res) => eprintln!("resolve {root}: {}", res.err),
				Err(e) => eprintln!("hub resolve: {e}"),
			}
		}
		Some(super::HubAction::Status) => {
			let client = match HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
				Ok(c) => c,
				Err(e) => {
					eprintln!("hub: not running ({e})");
					return;
				}
			};
			match client.status().await {
				Ok(res) => {
					if res.nodes.is_empty() {
						println!("hub: running, no nodes");
					}
					for n in res.nodes {
						println!(
							"{}  pid:{}  {}  {}",
							if n.alive { "up  " } else { "dead" },
							n.pid,
							n.root,
							n.endpoint
						);
					}
				}
				Err(e) => eprintln!("hub status: {e}"),
			}
		}
		Some(super::HubAction::Unload { root }) => {
			let root = root.unwrap_or_else(default_root);
			let client = match HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
				Ok(c) => c,
				Err(e) => {
					eprintln!("hub: not running ({e})");
					return;
				}
			};
			match client.unload(UnloadReq { root: root.clone() }).await {
				Ok(res) if res.ok && res.existed => println!("unloaded {root}"),
				Ok(res) if res.ok => println!("no node for {root}"),
				Ok(res) => eprintln!("unload {root}: {}", res.err),
				Err(e) => eprintln!("hub unload: {e}"),
			}
		}
		Some(super::HubAction::Merge { src, dst }) => cmd_hub_merge(&src, &dst).await,
		Some(super::HubAction::Stop) => match HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
			Ok(client) => match client.stop().await {
				Ok(_) => println!("hub stopped (nodes stay up)"),
				Err(e) => eprintln!("hub stop: {e}"),
			},
			Err(e) => eprintln!("hub: not running ({e})"),
		},
	}
}

// Offline CRDT union: src's rows and topology join dst's store; src is never
// written. Both daemons must be down — the store is single-writer and a live
// daemon's flush would clobber the merge.
async fn cmd_hub_merge(src: &str, dst: &str) {
	use trnsprt::hub_rpc::{HubRpcClient, UnloadReq};
	use trnsprt::typed::JsonEnvelopeCodec;

	let canon = |s: &str| -> Option<std::path::PathBuf> {
		let p = std::path::Path::new(s).canonicalize().ok()?;
		Some(crate::config::Config::resolve_root(&p))
	};
	let Some(src_root) = canon(src) else {
		eprintln!("merge: src {src} does not exist");
		return;
	};
	let Some(dst_root) = canon(dst) else {
		eprintln!("merge: dst {dst} does not exist");
		return;
	};
	if src_root == dst_root {
		eprintln!(
			"merge: src and dst are the same root {}",
			src_root.display()
		);
		return;
	}
	if !src_root.join(".kern").is_dir() {
		eprintln!("merge: src {} has no .kern store", src_root.display());
		return;
	}

	if let Ok(client) = HubRpcClient::<JsonEnvelopeCodec>::connect_hub().await {
		for root in [&src_root, &dst_root] {
			let _ = client
				.unload(UnloadReq {
					root: root.display().to_string(),
				})
				.await;
		}
	}
	for root in [&src_root, &dst_root] {
		if crate::hub::node::probe(root).await {
			eprintln!(
				"merge: a daemon still serves {} — stop it first",
				root.display()
			);
			return;
		}
	}

	// Fallback must stay pinned to the root: a bare `Config::default()` carries a
	// cwd-relative data_dir and would read (and write!) whatever store the
	// caller happens to stand in.
	let src_cfg = crate::config::Config::load(&src_root)
		.unwrap_or_else(|_| crate::config::Config::default_in(&src_root));
	let dst_cfg = crate::config::Config::load(&dst_root)
		.unwrap_or_else(|_| crate::config::Config::default_in(&dst_root));
	let src_g = load_graph(&src_cfg);
	let mut dst_g = load_graph(&dst_cfg);

	let src_h = crate::base::health::graph_health_stats(&src_g);
	if src_h.entities == 0 {
		eprintln!("merge: src {} holds no entities", src_root.display());
		return;
	}
	let before = crate::base::health::graph_health_stats(&dst_g);
	let changed = crate::base::merge::absorb_graph(&mut dst_g, src_g);
	save_graph_unguarded(&dst_g);
	let after = crate::base::health::graph_health_stats(&dst_g);
	println!(
		"merged {} -> {}: {} rows joined, entities {} -> {}, kerns {} -> {} (src untouched)",
		src_root.display(),
		dst_root.display(),
		changed,
		before.entities,
		after.entities,
		before.kerns,
		after.kerns,
	);
}

#[cfg(test)]
mod hub_merge_tests {
	use crate::base::types::{mk_entity, EntityKind, Kern};

	fn store_with_entity(root: &std::path::Path, eid: &str) {
		std::fs::create_dir_all(root.join(".kern")).unwrap();
		let cfg = crate::config::Config::default_in(root);
		let mut g = crate::base::graph::GraphGnn::new();
		g.data_dir = cfg.data_dir.clone();
		std::fs::create_dir_all(&g.data_dir).unwrap();
		let mut k = Kern::new("k-hub-merge", g.root.id.clone());
		k.root_id = g.root.id.clone();
		k.graviton_text = "merge test".into();
		k.entities.insert(
			eid.to_string(),
			mk_entity(eid, "merged fact", 1.0, EntityKind::Fact),
		);
		g.register(k);
		// save_all silently no-ops without a store attached.
		let store = crate::base::store::Store::open(&g.data_dir).unwrap();
		g.set_store(std::sync::Arc::new(store));
		crate::base::persist::save_all(&g).unwrap();
	}

	fn dst_entities(root: &std::path::Path) -> usize {
		let cfg = crate::config::Config::default_in(root);
		let g = super::load_graph(&cfg);
		crate::base::health::graph_health_stats(&g).entities
	}

	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn merge_absorbs_src_entities_into_dst_and_leaves_src_alone() {
		let dir = tempfile::tempdir().unwrap();
		let src = dir.path().join("src");
		let dst = dir.path().join("dst");
		store_with_entity(&src, "e-src");
		store_with_entity(&dst, "e-dst");
		assert_eq!(dst_entities(&src), 1, "src store persisted before merge");

		super::cmd_hub_merge(&src.display().to_string(), &dst.display().to_string()).await;

		assert_eq!(
			dst_entities(&dst),
			2,
			"dst holds its own + the absorbed entity"
		);
		assert_eq!(dst_entities(&src), 1, "src is never written");
	}

	#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
	async fn merge_refuses_identical_roots_and_missing_src() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("only");
		store_with_entity(&root, "e-1");
		let r = root.display().to_string();

		// Same root: refused before any store is touched.
		super::cmd_hub_merge(&r, &r).await;
		assert_eq!(dst_entities(&root), 1, "self-merge is a refused no-op");

		// Missing src: refused.
		super::cmd_hub_merge("/nonexistent/kern-merge-src", &r).await;
		assert_eq!(dst_entities(&root), 1, "missing src leaves dst untouched");
	}
}
