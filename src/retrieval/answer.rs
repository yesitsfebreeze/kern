use crate::base::constants::{ANSWER_MAX_CHAINS, ANSWER_MAX_THOUGHTS, REFINE_INTERVAL};
use crate::base::graph::GraphGnn;
use crate::base::search::{find_entity, find_reason};
use crate::base::util;
use crate::config::RetrievalConfig;
use crate::profile::Profiler;
use crate::retrieval::expand::{self, PathChain, ScoredEntity, ScoredRef};
use crate::retrieval::score::{self, QueryOptions};
use crate::retrieval::seed::{self, Mode, Weights};
use crate::retrieval::{diversify, fuse, gravity, hyde, merge, pagerank, rerank, LlmFunc};

// Must stay inside locomo::is_abstention's marker set — pinned by a test there.
pub const NO_ANSWER: &str = "I don't have information about that.";

#[derive(Debug, Clone)]
pub struct QueryResult {
	pub answer: String,
	pub entities: Vec<ScoredEntity>,
	pub path_chains: Vec<PathChain>,
}

#[allow(clippy::too_many_arguments)]
pub fn query(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f32],
	query_text: &str,
	mode: Mode,
	llm: Option<&LlmFunc>,
	embedder_fn: Option<&crate::retrieval::EmbedFunc>,
	opts: Option<QueryOptions>,
) -> QueryResult {
	let (result, profile) =
		query_profiled(g, cfg, query_vec, query_text, mode, llm, embedder_fn, opts);
	tracing::debug!(target: "kern.profile", "{}", profile);
	result
}

#[allow(clippy::too_many_arguments)]
pub fn query_profiled(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &[f32],
	query_text: &str,
	mode: Mode,
	llm: Option<&LlmFunc>,
	embedder_fn: Option<&crate::retrieval::EmbedFunc>,
	opts: Option<QueryOptions>,
) -> (QueryResult, crate::profile::Profile) {
	let mut prof = Profiler::new("query");
	let w = Weights::for_mode(cfg, mode);

	let fused_qvec = hyde::expand_query(cfg, llm, embedder_fn, query_vec, query_text);
	prof.checkpoint("hyde");

	let Retrieved {
		mut results,
		chains,
		chain_text,
	} = retrieve(g, cfg, &fused_qvec, query_text, mode, opts.as_ref(), w);
	prof.checkpoint("retrieve");

	rerank::llm_rerank(cfg, llm, query_text, &mut results);
	prof.checkpoint("rerank");

	score::commit_access(&mut results);

	let style = opts.as_ref().and_then(|o| o.answer_style.as_deref());
	let answer = synthesize(&chain_text, &results, query_text, llm, style);
	prof.checkpoint("answer");

	(
		QueryResult {
			answer,
			entities: results,
			path_chains: chains,
		},
		prof.finish(),
	)
}

// chain_text is pre-rendered while the graph lock is held, so the answer prompt needs no graph access afterward.
pub struct Retrieved {
	pub results: Vec<ScoredEntity>,
	pub chains: Vec<PathChain>,
	pub chain_text: String,
}

fn fuse_hybrid_seeds(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	opts: Option<&QueryOptions>,
	lex: &crate::base::lexical::LexicalIndex,
	dense_seeds: Vec<crate::base::search::EntityHit>,
	query_text: &str,
	imp_hits: &[crate::base::search::EntityHit],
) -> Vec<crate::base::search::EntityHit> {
	let lex_hits = seed::seed_lexical(lex, g, query_text, cfg.seed_k * 4, opts);
	let pr_hits = if cfg.pagerank_enabled {
		// Teleport personalized at dense + lexical seeds only — importance is query-independent and would make PageRank query-blind.
		let ppr_seeds: Vec<crate::base::search::EntityHit> =
			dense_seeds.iter().chain(lex_hits.iter()).cloned().collect();
		pagerank::pagerank(
			g,
			&ppr_seeds,
			cfg.pagerank_damping,
			cfg.pagerank_iters,
			cfg.pagerank_top_k,
		)
	} else {
		Vec::new()
	};
	let gw = cfg.rrf_global_weight;
	let mut lists: Vec<&[crate::base::search::EntityHit]> = vec![&dense_seeds, &lex_hits, imp_hits];
	let mut weights: Vec<f64> = vec![1.0, 1.0, gw];
	if !pr_hits.is_empty() {
		lists.push(&pr_hits);
		weights.push(gw);
	}
	fuse::rrf(&lists, &weights, cfg.rrf_k, cfg.seed_k.max(1) * 2)
}

// Graph-only half of retrieval (seed -> expand -> merge -> score -> diversify). NO LLM — callers hold the graph lock for exactly this sub-ms phase.
#[allow(clippy::too_many_arguments)]
pub fn retrieve(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	qvec: &[f32],
	query_text: &str,
	mode: Mode,
	opts: Option<&QueryOptions>,
	w: Weights,
) -> Retrieved {
	retrieve_profiled(g, cfg, qvec, query_text, mode, opts, w).0
}

#[allow(clippy::too_many_arguments)]
pub fn retrieve_profiled(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	qvec: &[f32],
	query_text: &str,
	mode: Mode,
	opts: Option<&QueryOptions>,
	w: Weights,
) -> (Retrieved, crate::profile::Profile) {
	let mut prof = Profiler::new("retrieve");
	let lexical = g.lexical();
	let lex_ref = lexical.as_deref();
	// The O(N) importance scan feeds both the dense-seed merge and the RRF list — run once here, threaded into both.
	let important = seed::seed_important(g, cfg, qvec, opts);
	let dense_seeds = seed::seed_with_important(g, cfg, qvec, cfg.seed_k, mode, opts, &important);
	prof.checkpoint("seed_dense");

	let seeds = if mode == Mode::Hybrid && cfg.lexical_enabled && !query_text.is_empty() {
		match lex_ref {
			Some(lex) => fuse_hybrid_seeds(g, cfg, opts, lex, dense_seeds, query_text, &important),
			None => dense_seeds,
		}
	} else {
		dense_seeds
	};
	prof.checkpoint("fuse_hybrid");

	if seeds.is_empty() {
		return (
			Retrieved {
				results: Vec::new(),
				chains: Vec::new(),
				chain_text: String::new(),
			},
			prof.finish(),
		);
	}

	let expanded = expand::expand(g, cfg, qvec, &seeds, w);
	prof.checkpoint("expand");
	let mut results = merge::merge(g, &seeds, expanded.scored);
	let chains = expanded.chains;
	prof.checkpoint("merge");

	score::apply_boosts(cfg, &mut results);
	gravity::apply_gravity(g, cfg, &mut results);
	// An active filter must run BEFORE filter_delivery's pool truncation, or expansion's non-matching neighbours crowd matching entities out of the cap.
	if let Some(o) = opts {
		if o.is_active() {
			results.retain(|r| score::matches_filter(r.entity, o));
		}
	}
	score::filter_delivery(cfg, &mut results);

	if let Some(opts) = opts {
		score::apply_query_options(&mut results, opts);
	}

	diversify::dedup_by_section(cfg, &mut results);
	prof.checkpoint("boosts+filter");
	diversify::mmr(cfg, qvec, &mut results);
	prof.checkpoint("mmr");

	let results: Vec<ScoredEntity> = results.into_iter().map(ScoredRef::to_owned).collect();
	prof.checkpoint("materialize");

	let chain_text = format_chains(g, &chains);
	prof.checkpoint("chains");
	(
		Retrieved {
			results,
			chains,
			chain_text,
		},
		prof.finish(),
	)
}

// No graph access — callable after the lock is released.
pub fn synthesize(
	chain_text: &str,
	scored: &[ScoredEntity],
	query_text: &str,
	llm: Option<&LlmFunc>,
	style: Option<&str>,
) -> String {
	if query_text.is_empty() {
		return String::new();
	}
	match llm {
		Some(llm_fn) => {
			// Empty context: abstain without an LLM call — cheaper and more reliable
			// than asking the model to notice it has nothing to work with.
			if scored.is_empty() && chain_text.is_empty() {
				return NO_ANSWER.to_string();
			}
			let mut prompt = answer_prompt_from(chain_text, scored, query_text);
			if let Some(s) = style {
				prompt.push(' ');
				prompt.push_str(s);
			}
			llm_fn(&prompt)
		}
		None => String::new(),
	}
}

// Holds the read lock for ONLY the graph phase; every LLM call runs unlocked. Daemon MCP path; plain query() serves the one-shot CLI.
#[allow(clippy::too_many_arguments)]
pub fn query_locked(
	graph: &parking_lot::RwLock<GraphGnn>,
	cfg: &RetrievalConfig,
	query_vec: &[f32],
	query_text: &str,
	mode: Mode,
	llm: Option<&LlmFunc>,
	embedder_fn: Option<&crate::retrieval::EmbedFunc>,
	opts: Option<QueryOptions>,
) -> (QueryResult, u64) {
	let w = Weights::for_mode(cfg, mode);

	// HyDE LLM call — graph-free, so do it before taking any lock.
	let fused_qvec = hyde::expand_query(cfg, llm, embedder_fn, query_vec, query_text);

	// Epoch captured under the SAME lock as retrieval: a write during the lock-free
	// LLM phase leaves the cache stamp born stale → miss, never a stale serve.
	let (mut retrieved, epoch) = {
		let g = crate::base::locks::read_recovered(graph);
		let r = retrieve(&g, cfg, &fused_qvec, query_text, mode, opts.as_ref(), w);
		(r, g.mutation_epoch())
	};

	rerank::llm_rerank(cfg, llm, query_text, &mut retrieved.results);
	score::commit_access(&mut retrieved.results);
	// Live-graph access write-back is deferred to a CommitAccess tick task (see
	// mcp::Server::tool_query) so this path takes ONLY a read lock (see note).
	let style = opts.as_ref().and_then(|o| o.answer_style.as_deref());
	let answer = synthesize(
		&retrieved.chain_text,
		&retrieved.results,
		query_text,
		llm,
		style,
	);

	(
		QueryResult {
			answer,
			entities: retrieved.results,
			path_chains: retrieved.chains,
		},
		epoch,
	)
}

pub fn build_answer_prompt(
	g: &GraphGnn,
	chains: &[PathChain],
	scored: &[ScoredEntity],
	query_text: &str,
) -> String {
	answer_prompt_from(&format_chains(g, chains), scored, query_text)
}

pub fn answer_prompt_from(chain_text: &str, scored: &[ScoredEntity], query_text: &str) -> String {
	let mut prompt = String::from("Context from knowledge graph:\n\n");
	if !chain_text.is_empty() {
		prompt.push_str(chain_text);
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
		 Do not restate the context. Be direct. \
		 If the context does not contain the answer, say exactly: {NO_ANSWER}"
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
				let label = if !r.text.is_empty() {
					util::truncate(&r.text, 100).to_string()
				} else if let Some(lbl) = r.kind.fallback_label() {
					lbl.to_string()
				} else {
					continue;
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
			let (reason, kern_id) = match find_reason(g, node_id) {
				Some(pair) => pair,
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
					let lamport = g.bump_lamport();
					let producer = g.network_id.clone();
					if let Some(kern) = g.get_mut(&kern_id) {
						if let Some(r) = kern.reasons.get_mut(node_id) {
							r.score = clamped;
							r.score_lamport = lamport;
							r.score_producer = producer.clone();
							let lww_value = bincode::serde::encode_to_vec(clamped, bincode::config::standard())
								.unwrap_or_default();
							g.push_delta(crate::base::graph::PendingDelta {
								object_id: node_id.clone(),
								target: 2,
								replica: String::new(),
								value: 0,
								lamport,
								producer,
								lww_value,
							});
						}
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;
	use crate::base::types::{mk_entity, EntityKind, Kern, Reason, ReasonKind};
	use std::sync::Arc;

	fn scored(id: &str, text: &str, score: f64) -> ScoredEntity {
		ScoredEntity {
			entity: mk_entity(id, text, 0.0, EntityKind::Claim),
			score,
		}
	}

	#[test]
	fn synthesize_is_empty_without_a_query_or_an_llm() {
		let s = [scored("a", "fact", 1.0)];
		assert!(
			synthesize("ctx", &s, "", None, None).is_empty(),
			"empty query -> empty answer"
		);
		assert!(
			synthesize("ctx", &s, "q?", None, None).is_empty(),
			"no llm -> empty answer"
		);
	}

	#[test]
	fn synthesize_abstains_on_empty_context_without_calling_the_llm() {
		let llm: LlmFunc = Arc::new(|_: &str| panic!("LLM must not run on empty context"));
		let out = synthesize("", &[], "q?", Some(&llm), None);
		assert_eq!(out, NO_ANSWER);
	}

	#[test]
	fn synthesize_appends_the_style_hint_to_the_prompt() {
		let s = [scored("a", "fact", 1.0)];
		let seen = Arc::new(std::sync::Mutex::new(String::new()));
		let seen2 = seen.clone();
		let llm: LlmFunc = Arc::new(move |p: &str| {
			*seen2.lock().unwrap() = p.to_string();
			"ok".to_string()
		});
		synthesize("", &s, "q?", Some(&llm), Some("STYLE-HINT"));
		assert!(seen.lock().unwrap().ends_with("STYLE-HINT"));
	}

	#[test]
	fn synthesize_calls_the_llm_with_the_assembled_prompt() {
		let s = [scored("a", "the sky is blue", 1.0)];
		let seen = Arc::new(std::sync::Mutex::new(String::new()));
		let seen2 = seen.clone();
		let llm: LlmFunc = Arc::new(move |p: &str| {
			*seen2.lock().unwrap() = p.to_string();
			"blue".to_string()
		});
		let out = synthesize("CHAINS", &s, "what colour?", Some(&llm), None);
		assert_eq!(out, "blue", "llm output returned verbatim");
		let prompt = seen.lock().unwrap();
		assert!(prompt.contains("what colour?"), "query in prompt: {prompt}");
		assert!(prompt.contains("the sky is blue"), "fact in prompt");
		assert!(prompt.contains("CHAINS"), "chain text in prompt");
	}

	#[test]
	fn answer_prompt_from_numbers_facts_and_appends_the_question() {
		let s = [
			scored("a", "first fact", 1.0),
			scored("b", "second fact", 0.9),
		];
		let p = answer_prompt_from("", &s, "why?");
		assert!(p.starts_with("Context from knowledge graph:"));
		assert!(p.contains("1. first fact"));
		assert!(p.contains("2. second fact"));
		assert!(p.contains("Question: why?"));
	}

	#[test]
	fn answer_prompt_from_inlines_chain_text_when_present() {
		let p = answer_prompt_from("Chain 1:\n  [Entity] x\n", &[], "q");
		assert!(
			p.contains("Chain 1:"),
			"chain text inlined ahead of the facts"
		);
	}

	#[test]
	fn format_chains_renders_entities_and_reason_labels() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert(
			"e1".into(),
			mk_entity("e1", "alpha", 0.0, EntityKind::Claim),
		);
		k.entities
			.insert("e2".into(), mk_entity("e2", "beta", 0.0, EntityKind::Claim));
		add_reason(
			&mut k,
			Reason {
				from: "e1".into(),
				to: "e2".into(),
				id: "r1".into(),
				text: "supports".into(),
				kind: ReasonKind::Similarity,
				..Default::default()
			},
		);
		g.kerns.insert("k".into(), k);

		let chains = [PathChain {
			nodes: vec!["e1".into(), "r1".into(), "e2".into()],
			score: 1.0,
		}];
		let out = format_chains(&g, &chains);
		assert!(out.contains("Chain 1:"));
		assert!(out.contains("[Entity] alpha"));
		assert!(out.contains("[Entity] beta"));
		assert!(
			out.contains("--supports-->"),
			"reason text used as the edge label: {out}"
		);
	}

	#[test]
	fn query_locked_is_read_only_and_defers_the_access_stamp() {
		use crate::base::accept;
		use parking_lot::RwLock;

		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let mut e = mk_entity("hot", "the sky is blue", 0.0, EntityKind::Claim);
		e.vector = vec![1.0, 0.0, 0.0, 0.0];
		accept::accept(&mut g, &root, e, "");
		let graph = RwLock::new(g);

		let cfg = RetrievalConfig::default();
		let (result, _) = query_locked(
			&graph,
			&cfg,
			&[1.0, 0.0, 0.0, 0.0],
			"sky",
			crate::retrieval::seed::Mode::Content,
			None,
			None,
			None,
		);
		assert!(!result.entities.is_empty(), "the entity is retrieved");
		assert!(
			result.entities.iter().any(|s| s.entity.id == "hot"),
			"the caller gets the retrieved id so it can enqueue the deferred stamp"
		);

		let g = graph.read();
		let (live, _) = find_entity(&g, "hot").expect("entity still live");
		assert!(
			live.accessed_at.is_none(),
			"query_locked does NOT stamp the live graph — the write-back is deferred"
		);
		assert_eq!(
			live.access_count.value(),
			0,
			"no inline write lock: the live access counter is untouched by the read path"
		);
	}

	#[test]
	fn build_answer_prompt_wraps_facts_and_the_question() {
		let g = GraphGnn::new();
		let s = [scored("a", "the fact", 1.0)];
		let p = build_answer_prompt(&g, &[], &s, "ask?");
		assert!(p.contains("1. the fact"));
		assert!(p.contains("Question: ask?"));
	}
}
