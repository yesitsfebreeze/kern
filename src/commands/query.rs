use crate::base::search::{find_entity, search_all_unlocked};
use crate::base::util::{short_id, truncate};

use super::{Client, Endpoint, load_graph, save_graph};

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
	let llm_client = Client::new(
		Endpoint::new(reason_url, reason_model, cfg.reason_key()),
		Endpoint::new(cfg.answer_url(), &cfg.answer.model, cfg.answer_key()),
		Endpoint::new(embed_url, embed_model, &cfg.embed.key),
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
		use futures_util::StreamExt as _;
		let prompt = crate::retrieval::answer::build_answer_prompt(
			&g,
			&result.path_chains,
			&result.entities,
			text,
		);
		// Single-shot (stream:false): one round-trip, collected into the printed
		// answer. The streamed tokens arrive through the same interface the `/ask`
		// UI consumes incrementally — see `Client::answer`.
		let mut gen = std::pin::pin!(llm_client.answer(crate::llm::AnswerParams {
			messages: vec![("user".to_string(), prompt)],
			stream: false,
			num_predict: None,
		}));
		println!("--- Answer ---");
		while let Some(item) = gen.next().await {
			match item {
				Ok(tok) => print!("{tok}"),
				Err(e) => {
					eprintln!("answer: {e}");
					return;
				}
			}
		}
		println!();
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
	let llm_client = Client::new(
		Endpoint::default(),
		Endpoint::default(),
		Endpoint::new(embed_url, embed_model, &cfg.embed.key),
	);
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
