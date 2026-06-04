use crate::base::constants::{ANSWER_MAX_CHAINS, ANSWER_MAX_THOUGHTS, REFINE_INTERVAL};
use crate::base::graph::GraphGnn;
use crate::base::search::{find_reason, find_entity};
use crate::base::util;
use crate::config::RetrievalConfig;
use crate::retrieval::expand::{self, PathChain, ScoredEntity};
use crate::retrieval::score::{self, QueryOptions};
use crate::retrieval::seed::{self, Mode, Weights};
use crate::retrieval::{diversify, fuse, hyde, merge, pagerank, rerank, LlmFunc};

#[derive(Debug, Clone)]
pub struct QueryResult {
	pub answer: String,
	pub entities: Vec<ScoredEntity>,
	pub path_chains: Vec<PathChain>,
}

pub fn query(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f64],
	query_text: &str,
	mode: Mode,
	llm: Option<&LlmFunc>,
	embedder_fn: Option<&crate::retrieval::EmbedFunc>,
	opts: Option<QueryOptions>,
) -> QueryResult {
	let w = Weights::for_mode(cfg, mode);

	let fused_qvec = hyde::expand_query(cfg, llm, embedder_fn, query_vec, query_text);
	let qvec: &[f64] = &fused_qvec;

	let lexical = g.lexical();
	let lex_ref = lexical.as_deref();
	let dense_seeds = seed::seed(g, cfg, qvec, query_text, cfg.seed_k, mode, lex_ref);

	let seeds = if mode == Mode::Hybrid && cfg.lexical_enabled && !query_text.is_empty() {
		if let Some(lex) = lex_ref {
			let lex_hits = seed::seed_lexical(lex, query_text, cfg.seed_k * 4);
			let imp_hits = seed::seed_important(g, cfg, qvec);
			let pr_hits = if cfg.pagerank_enabled {
				pagerank::pagerank(
					g,
					cfg.pagerank_damping,
					cfg.pagerank_iters,
					cfg.pagerank_top_k,
				)
			} else {
				Vec::new()
			};
			let mut lists: Vec<&[crate::base::search::EntityHit]> =
				vec![&dense_seeds, &lex_hits, &imp_hits];
			if !pr_hits.is_empty() {
				lists.push(&pr_hits);
			}
			let fused = fuse::rrf(&lists, cfg.rrf_k, cfg.seed_k.max(1) * 2);
			if fused.is_empty() {
				dense_seeds
			} else {
				fused
			}
		} else {
			dense_seeds
		}
	} else {
		dense_seeds
	};

	if seeds.is_empty() {
		return QueryResult {
			answer: String::new(),
			entities: Vec::new(),
			path_chains: Vec::new(),
		};
	}

	let expanded = expand::expand(g, cfg, qvec, &seeds, w);

	let mut results = merge::merge(g, &seeds, expanded.scored);
	let chains = expanded.chains;

	score::apply_boosts(cfg, &mut results);
	score::filter_delivery(cfg, &mut results);

	if let Some(ref opts) = opts {
		score::apply_query_options(&mut results, opts);
	}

	diversify::dedup_by_section(cfg, &mut results);
	diversify::mmr(cfg, qvec, &mut results);

	rerank::llm_rerank(cfg, llm, query_text, &mut results);

	score::commit_access(&mut results);

	let answer = if !query_text.is_empty() {
		if let Some(llm_fn) = llm {
			let prompt = build_answer_prompt(g, &chains, &results, query_text);
			llm_fn(&prompt)
		} else {
			String::new()
		}
	} else {
		String::new()
	};

	QueryResult {
		answer,
		entities: results,
		path_chains: chains,
	}
}

pub fn build_answer_prompt(
	g: &GraphGnn,
	chains: &[PathChain],
	scored: &[ScoredEntity],
	query_text: &str,
) -> String {
	let mut prompt = String::from("Context from knowledge graph:\n\n");
	let chain_text = format_chains(g, chains);
	if !chain_text.is_empty() {
		prompt.push_str(&chain_text);
		prompt.push('\n');
	}
	prompt.push_str("Relevant facts:\n");
	for (i, st) in scored.iter().take(ANSWER_MAX_THOUGHTS).enumerate() {
		let text = st.entity.text();
		let truncated = util::truncate(&text, 300);
		prompt.push_str(&format!("{}. {}\n", i + 1, truncated));
	}
	prompt.push_str(&format!(
		"\nQuestion: {query_text}\n\
		 Answer the question concisely using only the context above. \
		 Do not restate the context. Be direct."
	));
	prompt
}

pub fn format_chains(g: &GraphGnn, chains: &[PathChain]) -> String {
	let mut out = String::new();
	for (i, chain) in chains.iter().take(ANSWER_MAX_CHAINS).enumerate() {
		out.push_str(&format!("Chain {}:\n", i + 1));
		for (j, node_id) in chain.nodes.iter().enumerate() {
			if j % 2 == 0 {
				if let Some((t, _)) = find_entity(g, node_id) {
					let text = util::truncate(&t.text(), 200);
					out.push_str(&format!("  [Entity] {text}\n"));
				}
			} else if let Some((r, _)) = find_reason(g, node_id) {
				let label = if r.text.is_empty() {
					format!("{:?}", r.kind)
				} else {
					util::truncate(&r.text, 100).to_string()
				};
				out.push_str(&format!("  --{label}-->\n"));
			}
		}
	}
	out
}

pub fn refine_edges(g: &mut GraphGnn, chains: &[PathChain], llm: &LlmFunc) {
	for chain in chains {
		for (j, node_id) in chain.nodes.iter().enumerate() {
			if j.is_multiple_of(2) {
				continue;
			}
			let reason = match find_reason(g, node_id) {
				Some((r, _)) => r,
				None => continue,
			};
			let tc = reason.traversal_count.value();
			if tc > 0 && (tc as u32).is_multiple_of(REFINE_INTERVAL) {
				let from_text = find_entity(g, &reason.from)
					.map(|(t, _)| t.text())
					.unwrap_or_default();
				let to_text = find_entity(g, &reason.to)
					.map(|(t, _)| t.text())
					.unwrap_or_default();

				if from_text.is_empty() || to_text.is_empty() {
					continue;
				}

				let prompt = format!(
					"Rate the strength of the relationship between these two knowledge items \
					 on a scale from 0.0 to 1.0. Respond with only the number.\n\n\
					 A: {}\n\nB: {}",
					util::truncate(&from_text, 200),
					util::truncate(&to_text, 200),
				);
				let response = llm(&prompt);
				if let Ok(new_score) = response.trim().parse::<f64>() {
					let clamped = new_score.clamp(0.0, 1.0);
					for kern_id in g.all_ids() {
						if let Some(kern) = g.get_mut(&kern_id) {
							if let Some(r) = kern.reasons.get_mut(node_id) {
								r.score = clamped;
								break;
							}
						}
					}
				}
			}
		}
	}
}
