use crate::base::search::{find_entity, search_all_unlocked};
use crate::base::util::{short_id, truncate};

use super::{load_graph, Client};

pub(super) struct QueryParams<'a> {
	pub(super) text: &'a str,
	pub(super) mode: &'a str,
	pub(super) embed_url: &'a str,
	pub(super) embed_model: &'a str,
}

pub(super) async fn cmd_query(cfg: &crate::config::Config, params: QueryParams<'_>) {
	let QueryParams {
		text,
		mode,
		embed_url,
		embed_model,
	} = params;
	let g = load_graph(cfg);
	// Retrieval is LLM-free: only the embedder is needed.
	let llm_client = Client::new_embed_only(embed_url, embed_model, &cfg.embed.key);

	let vec = match llm_client.embed(text).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("embed: {e}");
			return;
		}
	};

	let mode = crate::retrieval::seed::Mode::parse(mode);

	let result = crate::retrieval::query::query(&g, &cfg.retrieval, &cfg.heat, &vec, text, mode, None);
	// No save: read-only — access/heat bumps land on cloned result entities, and
	// persisting would risk clobbering a daemon's newer on-disk state.

	if result.entities.is_empty() {
		println!("no results");
		return;
	}
	for (i, st) in result.entities.iter().enumerate() {
		println!(
			"{}. [{:.4}] {}  {}",
			i + 1,
			st.score,
			short_id(&st.entity.id),
			truncate(&st.entity.text(), 120),
		);
	}

	let chain_text = crate::retrieval::query::format_chains(&g, &result.path_chains);
	if !chain_text.trim().is_empty() {
		println!("\n--- Connections ---");
		print!("{chain_text}");
	}
}

pub(super) async fn cmd_search(
	cfg: &crate::config::Config,
	text: &str,
	k: usize,
	embed_url: &str,
	embed_model: &str,
) {
	let g = load_graph(cfg);
	// Reason deliberately unconfigured: pure vector retrieval never calls
	// them — do NOT "fix" these to real endpoints/credentials.
	let llm_client = Client::new_embed_only(embed_url, embed_model, &cfg.embed.key);
	let vec = match llm_client.embed(text).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("embed: {e}");
			return;
		}
	};

	let hits = search_all_unlocked(&g, &vec, k);
	if hits.is_empty() {
		println!("no results");
		return;
	}
	for (i, hit) in hits.iter().enumerate() {
		let text = find_entity(&g, &hit.entity_id)
			.map(|(t, _)| truncate(&t.text(), 120))
			.unwrap_or_default();
		println!(
			"{}. [{:.4}] {}  {}",
			i + 1,
			hit.score,
			short_id(&hit.entity_id),
			text
		);
	}
}
