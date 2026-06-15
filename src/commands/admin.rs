use crate::base::util::short_id;

use super::{
	load_graph, save_graph, with_graph, AnchorAction, Client, DescriptorAction, Endpoint,
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

pub(super) fn cmd_health(cfg: &crate::config::Config) {
	let g = load_graph(cfg);
	// Headline counts via the shared roll-up (same one repl::do_health and the
	// MCP health tool use), so the aggregation logic lives in exactly one place.
	let h = crate::base::health::graph_health_stats(&g);

	println!("data_dir:    {}", g.data_dir);
	if h.anchors.is_empty() {
		println!("anchors:     (none)");
	} else {
		println!("anchors:     {}", h.anchors.join(", "));
	}
	println!("kerns:       {}", h.kerns);
	println!("thoughts:    {} (unnamed: {})", h.entities, h.unnamed);
	println!("reasons:     {}", h.reasons);
	println!("descriptors: {}", g.root.descriptors.len());

	for k in g.all() {
		let label = if k.anchor_text.is_empty() {
			"[unnamed]"
		} else {
			&k.anchor_text
		};
		println!(
			"  kern:{}  thoughts:{}  reasons:{}",
			label,
			k.entities.len(),
			k.reasons.len(),
		);
	}
}

/// Offline compaction: reap empty unnamed kerns, persist the result, then shrink
/// the on-disk env. Run with the daemon stopped — it loads from disk, GCs, and
/// saves, so a live daemon would race and re-persist the bloated in-memory graph.
/// Cheap, idempotent, safe to re-run. The daemon also reaps on startup; this
/// command is the one-shot full clean (reap + file shrink) without the daemon.
///
/// The reap deletes rows but LMDB never returns freed pages to the OS, so the
/// `data.mdb` file stays at its high-water mark (4 GiB on a runaway-bloated graph)
/// until it is rewritten. We drop the graph's env handle, then [`compact_dir`]
/// rewrites the live data into a fresh, gap-free file.
pub(super) fn cmd_gc(cfg: &crate::config::Config) {
	let mut g = load_graph(cfg);
	let (before, reaped, after) = g.gc_empty_kerns_counted();
	save_graph(&g);
	println!("gc: reaped {reaped} empty kerns ({before} -> {after})");

	// Drop the graph FIRST so its env handle is released, then compact. `compact_dir`
	// opens its own env and closes it deterministically via `prepare_for_closing().wait()`
	// — a plain drop is lazy on Windows and would leave data.mdb mmap'd past the swap.
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

/// Compact-only: shrink `data.mdb` to its live size without reaping. Daemon must
/// be stopped. Useful when the reap already ran but the file never shrank.
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

/// Terse human-readable byte size for the gc/compact reports.
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

pub(super) async fn cmd_anchor(cfg: &crate::config::Config, action: AnchorAction) {
	match action {
		AnchorAction::Add {
			name,
			text,
			embed_url,
			embed_model,
		} => {
			let url = embed_url.as_deref().unwrap_or(&cfg.embed.url);
			let model = embed_model.as_deref().unwrap_or(&cfg.embed.model);
			let llm_client = Client::new(
				Endpoint::default(),
				Endpoint::default(),
				Endpoint::new(url, model, &cfg.embed.key),
			);
			let vec = match llm_client.embed(&text).await {
				Ok(v) => v,
				Err(e) => {
					eprintln!("embed: {e}");
					return;
				}
			};
			with_graph(cfg, |g| crate::base::accept::add_anchor(g, &name, vec));
			println!("anchor added: {name}");
		}
		AnchorAction::List => {
			let g = load_graph(cfg);
			println!("anchors:");
			for cid in crate::base::accept::root_anchor_ids(&g) {
				if let Some(c) = g.loaded(&cid) {
					println!(
						"  {}  thoughts:{}  reasons:{}",
						c.anchor_text,
						c.entities.len(),
						c.reasons.len(),
					);
				}
			}
		}
		AnchorAction::Remove { name } => {
			let removed = with_graph(cfg, |g| crate::base::accept::remove_anchor(g, &name));
			if removed {
				println!("anchor removed: {name}");
			} else {
				eprintln!("anchor not found: {name}");
			}
		}
	}
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
		// A custom key, NOT one of the defaults that load_graph re-injects on every
		// load (e.g. "code") — otherwise Rm would appear to "fail" as the default
		// descriptor is re-added on the next load.
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
		// Exercises the load -> graph_health_stats roll-up -> per-kern print path.
		// The contract under test is "does not panic on an empty/new data dir".
		cmd_health(&cfg);
	}
}

pub(super) fn cmd_register(cfg: &crate::config::Config, path: &str) {
	// Copy the graph at `path` into THIS cwd's store. The loaded graph is bound to
	// the source store, so we write into a freshly opened destination store rather
	// than relying on `save_graph` (which would write back to the source).
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
