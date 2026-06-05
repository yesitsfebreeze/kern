//! `kern reembed`: re-embed the entire graph with the configured embedding
//! model. The embedding dimension is locked into the graph on first ingest, so
//! switching models (e.g. nomic-embed-text 768-d -> qwen3-embedding) requires
//! re-embedding every stored vector. Run this AFTER changing `[embed] model`
//! in kern.toml and with the daemon stopped (it writes the graph directly).

use std::collections::HashMap;
use std::path::Path;

use crate::base::math::average_vec;

use super::{build_llm, load_graph, save_graph};

const BATCH: usize = 64;

pub(super) async fn cmd_reembed(cfg: &crate::config::Config, embed_url: &str, embed_model: &str) {
	let mut g = load_graph(cfg);
	// Embed-only client (reason params unused here).
	let client = build_llm(embed_url, embed_model, &cfg.embed.key, "", "", "");

	// Collect every entity's text, graph-wide, in a stable order.
	let mut ids: Vec<String> = Vec::new();
	let mut texts: Vec<String> = Vec::new();
	for kern in g.kerns.values() {
		for e in kern.entities.values() {
			ids.push(e.id.clone());
			texts.push(e.text());
		}
	}
	if ids.is_empty() {
		println!("reembed: graph is empty, nothing to do");
		return;
	}
	println!("reembed: {} entities -> model '{embed_model}'", ids.len());

	let new_vecs = match embed_all(&client, &ids, &texts).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("reembed: aborted, graph unchanged: {e}");
			return;
		}
	};

	// Assign new vectors. gnn_vector is re-seeded from the raw embed; the GNN
	// tick refines it later (a stale-dimension gnn_vector would break its index).
	for kern in g.kerns.values_mut() {
		for e in kern.entities.values_mut() {
			if let Some(v) = new_vecs.get(&e.id) {
				e.vector = v.clone();
				e.gnn_vector = v.clone();
			}
		}
	}
	// Reason edge vectors are the mean of their endpoints — recompute from the
	// new entity vectors so the reason index matches the new dimension.
	for kern in g.kerns.values_mut() {
		for r in kern.reasons.values_mut() {
			if let (Some(fv), Some(tv)) = (new_vecs.get(&r.from), new_vecs.get(&r.to)) {
				r.vector = average_vec(fv, tv);
			}
		}
	}

	g.rebuild_index();
	save_graph(&g);
	println!("reembed: hot graph done ({} entities)", new_vecs.len());

	reembed_cold(cfg, &client).await;
	println!("reembed: complete — model is now '{embed_model}'");
}

async fn embed_all(
	client: &crate::llm::Client,
	ids: &[String],
	texts: &[String],
) -> Result<HashMap<String, Vec<f64>>, String> {
	let mut out: HashMap<String, Vec<f64>> = HashMap::with_capacity(ids.len());
	let mut done = 0usize;
	for chunk in texts.chunks(BATCH) {
		let vs = client
			.embed_batch(chunk)
			.await
			.map_err(|e| e.to_string())?;
		if vs.len() != chunk.len() {
			return Err(format!(
				"embed returned {} vectors for {} inputs",
				vs.len(),
				chunk.len()
			));
		}
		for v in vs {
			out.insert(ids[done].clone(), v);
			done += 1;
		}
		println!("  {done}/{ids_len}", ids_len = ids.len());
	}
	Ok(out)
}

/// Re-embed spilled cold-tier entities too — otherwise their old-dimension
/// vectors mismatch the query and cold search silently drops them. Best-effort.
async fn reembed_cold(cfg: &crate::config::Config, client: &crate::llm::Client) {
	let cold_dir = Path::new(&cfg.data_dir).join("cold");
	let mut cold = crate::base::cold::load_all(&cold_dir);
	if cold.is_empty() {
		return;
	}
	println!("reembed: {} cold entities", cold.len());
	let texts: Vec<String> = cold.iter().map(|e| e.text()).collect();
	let mut ok = true;
	for (i, chunk) in texts.chunks(BATCH).enumerate() {
		match client.embed_batch(chunk).await {
			Ok(vs) if vs.len() == chunk.len() => {
				for (j, v) in vs.into_iter().enumerate() {
					cold[i * BATCH + j].vector = v;
				}
			}
			_ => {
				ok = false;
				break;
			}
		}
	}
	if !ok {
		eprintln!("reembed: cold re-embed failed; cold tier left unchanged");
		return;
	}
	// Rewrite the cold store with the re-embedded entities.
	let store = cold_dir.join("cold.jsonl");
	let _ = std::fs::remove_file(&store);
	for e in &cold {
		crate::base::cold::spill(&cold_dir, e);
	}
	println!("reembed: cold tier done ({} entities)", cold.len());
}
