use crate::base::util::short_id;

use super::{
	AnchorAction, Client, DescriptorAction, Endpoint, UnnamedAction, load_graph, save_graph,
	with_graph,
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

	println!("data_dir:    {}", g.data_dir);
	let anchors: Vec<String> = crate::base::accept::root_anchor_ids(&g)
		.iter()
		.filter_map(|cid| g.loaded(cid))
		.map(|c| c.anchor_text.clone())
		.collect();
	if anchors.is_empty() {
		println!("anchors:     (none)");
	} else {
		println!("anchors:     {}", anchors.join(", "));
	}

	let kerns = g.all();
	let mut total_entities = 0usize;
	let mut total_reasons = 0usize;
	let mut unnamed = 0usize;
	for k in &kerns {
		total_entities += k.entities.len();
		total_reasons += k.reasons.len();
		if k.is_unnamed() {
			unnamed += 1;
		}
	}
	println!("kerns:       {}", kerns.len());
	println!("thoughts:    {} (unnamed: {})", total_entities, unnamed);
	println!("reasons:     {}", total_reasons);
	println!("descriptors: {}", g.root.descriptors.len());

	for k in &kerns {
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

/// Offline compaction: reap empty unnamed kerns and persist the result. Run with
/// the daemon stopped — it loads from disk, GCs, and saves, so a live daemon would
/// race and re-persist the bloated in-memory graph. Cheap, idempotent, safe to
/// re-run. The daemon also does this on startup; this command is for a one-shot
/// compaction without spinning up the full daemon.
pub(super) fn cmd_gc(cfg: &crate::config::Config) {
	let mut g = load_graph(cfg);
	let (before, reaped, after) = g.gc_empty_kerns_counted();
	save_graph(&g);
	println!("gc: reaped {reaped} empty kerns ({before} -> {after})");
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

pub(super) fn cmd_peers() {
	unimplemented_subcommand("federation");
}

/// Unified message for subcommands whose backing subsystem is not yet
/// wired up in the Rust port. One place to flip when the subsystem lands.
pub(super) fn unimplemented_subcommand(subsystem: &str) {
	println!("{subsystem} is not yet implemented in the Rust port.");
}

pub(super) fn cmd_register(cfg: &crate::config::Config, path: &str) {
	match crate::base::persist::load_dir(path) {
		Ok(mut g) => {
			g.data_dir = cfg.data_dir.clone();
			save_graph(&g);
			println!("registered {path}");
		}
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
