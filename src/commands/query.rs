use crate::base::search::{find_entity, search_all_unlocked};
use crate::base::util::{short_id, truncate};

use super::{build_llm, load_graph, save_graph};

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_query(
	cfg: &crate::config::Config,
	text: &str,
	mode: &str,
	answer: bool,
	embed_url: &str,
	embed_model: &str,
	reason_url: &str,
	reason_model: &str,
) {
	let g = load_graph(cfg);
	let llm_client = build_llm(
		embed_url,
		embed_model,
		&cfg.embed.key,
		reason_url,
		reason_model,
		cfg.reason_key(),
		"",
		"",
		"",
	);

	let vec = match llm_client.embed(text).await {
		Ok(v) => v,
		Err(e) => {
			eprintln!("embed: {e}");
			return;
		}
	};

	let mode = crate::retrieval::seed::Mode::parse(mode);

	let result = crate::retrieval::answer::query(
		&g,
		&cfg.retrieval,
		&vec,
		text,
		mode,
		None,
		None,
		None,
	);
	save_graph(&g);

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

	if answer {
		let prompt = crate::retrieval::answer::build_answer_prompt(
			&g,
			&result.path_chains,
			&result.entities,
			text,
		);
		match llm_client.complete(&prompt).await {
			Ok(ans) => {
				println!("--- Answer ---");
				println!("{ans}");
			}
			Err(e) => eprintln!("answer: {e}"),
		}
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
	let llm_client = build_llm(embed_url, embed_model, &cfg.embed.key, "", "", "", "", "", "");
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
