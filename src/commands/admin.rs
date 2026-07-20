use crate::base::util::short_id;

use super::{
	load_graph, save_graph, with_graph, Client, DescriptorAction, GravitonAction, UnnamedAction,
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

pub(super) fn cmd_health(cfg: &crate::config::Config) {
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
	println!("descriptors: {}", g.root.descriptors.len());

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

// Daemon must be stopped: a live daemon would race and re-persist the bloated graph.
pub(super) fn cmd_gc(cfg: &crate::config::Config) {
	let mut g = load_graph(cfg);
	let (before, reaped, after) = g.gc_empty_kerns_counted();
	save_graph(&g);
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

// Daemon must be stopped.
pub(super) fn cmd_compact(cfg: &crate::config::Config) {
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

pub(super) async fn cmd_graviton(cfg: &crate::config::Config, action: GravitonAction) {
	match action {
		GravitonAction::Add {
			name,
			text,
			mass,
			embed,
		} => {
			let (url, model) = embed.resolve(cfg);
			let llm_client = Client::new_embed_only(url, model, &cfg.embed.key);
			let vec = match llm_client.embed(&text).await {
				Ok(v) => v,
				Err(e) => {
					eprintln!("embed: {e}");
					return;
				}
			};
			let mass = mass.unwrap_or(1.0);
			with_graph(cfg, |g| {
				crate::base::accept::add_graviton_with_mass(g, &name, vec, mass)
			});
			println!("graviton added: {name} (mass {mass})");
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
			let removed = with_graph(cfg, |g| crate::base::accept::remove_graviton(g, &name));
			if removed {
				println!("graviton removed: {name}");
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

pub(super) fn cmd_descriptor(cfg: &crate::config::Config, action: DescriptorAction) {
	match action {
		DescriptorAction::Add { name, description } => {
			with_graph(cfg, |g| {
				g.root.descriptors.insert(name.clone(), description);
			});
			println!("descriptor added: {name}");
		}
		DescriptorAction::Rm { name } => {
			with_graph(cfg, |g| {
				g.root.descriptors.remove(&name);
			});
			println!("descriptor removed: {name}");
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

	#[test]
	fn descriptor_add_then_remove_persists_through_the_graph() {
		let (_dir, cfg) = temp_cfg();
		// A custom key, not a default: default keys re-inject on every load, so Rm
		// would appear to fail on the next load.
		let key = "custom_test_kind";

		cmd_descriptor(
			&cfg,
			DescriptorAction::Add {
				name: key.into(),
				description: "a custom kind".into(),
			},
		);
		let g = load_graph(&cfg);
		assert_eq!(
			g.root.descriptors.get(key).map(String::as_str),
			Some("a custom kind"),
			"Add persists the descriptor onto the root",
		);

		cmd_descriptor(&cfg, DescriptorAction::Rm { name: key.into() });
		let g = load_graph(&cfg);
		assert!(
			!g.root.descriptors.contains_key(key),
			"Rm removes the custom descriptor"
		);
	}

	#[test]
	fn cmd_health_runs_on_a_fresh_graph_without_panicking() {
		let (_dir, cfg) = temp_cfg();
		cmd_health(&cfg);
	}
}

pub(super) fn cmd_register(cfg: &crate::config::Config, path: &str) {
	// The loaded graph is bound to the SOURCE store, so write into a freshly
	// opened destination store — save_graph would write back to the source.
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
		let endpoint = trnsprt::typed::Endpoint::kern_for(root);
		if crate::hub::node::probe(&endpoint).await {
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
	save_graph(&dst_g);
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
