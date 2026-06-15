use crate::base::util;
use crate::config::RetrievalConfig;
use crate::retrieval::expand::ScoredEntity;
use crate::retrieval::LlmFunc;

/// Hard ceiling on candidates packed into one rerank prompt, independent of the
/// configured `rerank_pool_size`. Each candidate contributes ~300 chars; without
/// this cap a large configured pool (or result set) would balloon the prompt and
/// blow the model's context window and latency budget. 32 is ample headroom for
/// relevance reranking — the tail beyond it keeps its original order anyway.
const MAX_RERANK_CANDIDATES: usize = 32;

pub fn llm_rerank(
	cfg: &RetrievalConfig,
	llm: Option<&LlmFunc>,
	query_text: &str,
	results: &mut Vec<ScoredEntity>,
) {
	if !cfg.rerank_enabled || query_text.is_empty() {
		return;
	}
	let llm_fn = match llm {
		Some(f) => f,
		None => return,
	};
	// Bound the rerank window by the result count AND the prompt-size cap.
	let pool = cfg
		.rerank_pool_size
		.min(results.len())
		.min(MAX_RERANK_CANDIDATES);
	if pool < 2 {
		return;
	}

	let mut prompt = String::from(
		"You are re-ranking search results by relevance to a query. \
		Return ONLY a JSON array of integer indices in best-to-worst order, no prose, no decimal points. \
		Example: [2,0,1,3] — integers only, never [2.0,0.0,1.0,3.0]\n\n",
	);
	prompt.push_str(&format!("Query: {query_text}\n\nCandidates:\n"));
	for (i, st) in results.iter().take(pool).enumerate() {
		let text = st.entity.text();
		let truncated = util::truncate(&text, 300);
		prompt.push_str(&format!("[{i}] {truncated}\n"));
	}
	prompt.push_str("\nRanking (JSON array of indices):");

	let response = llm_fn(&prompt);
	let order = match parse_ranking(&response, pool) {
		Some(o) => o,
		None => return,
	};

	let tail = results.split_off(pool);
	let head = std::mem::take(results);
	let mut reordered: Vec<ScoredEntity> = Vec::with_capacity(pool);
	let mut used = vec![false; head.len()];
	for i in &order {
		if *i < head.len() && !used[*i] {
			used[*i] = true;
			reordered.push(head[*i].clone());
		}
	}
	for (i, st) in head.into_iter().enumerate() {
		if !used[i] {
			reordered.push(st);
		}
	}
	reordered.extend(tail);
	*results = reordered;
}

pub fn parse_ranking(response: &str, pool: usize) -> Option<Vec<usize>> {
	let trimmed = response.trim();
	let start = trimmed.find('[')?;
	let end = trimmed.rfind(']')?;
	if end <= start {
		return None;
	}
	let slice = &trimmed[start..=end];
	let arr: serde_json::Value = serde_json::from_str(slice).ok()?;
	let list = arr.as_array()?;
	let mut out = Vec::with_capacity(list.len());
	for v in list {
		// Accept integer JSON (1) or whole-number float JSON (1.0); reject fractions (1.5).
		let i = v
			.as_i64()
			.or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))? as usize;
		if i < pool {
			out.push(i);
		}
	}
	if out.is_empty() {
		None
	} else {
		Some(out)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::Entity;
	use std::sync::{Arc, Mutex};

	fn scored(id: &str, text: &str, score: f64) -> ScoredEntity {
		let mut e = Entity {
			id: id.into(),
			..Default::default()
		};
		e.set_text(text.into());
		ScoredEntity { entity: e, score }
	}

	fn cfg(pool: usize) -> RetrievalConfig {
		RetrievalConfig {
			rerank_enabled: true,
			rerank_pool_size: pool,
			..Default::default()
		}
	}

	fn ids(rs: &[ScoredEntity]) -> Vec<String> {
		rs.iter().map(|s| s.entity.id.clone()).collect()
	}

	#[test]
	fn llm_rerank_reorders_head_by_returned_ranking() {
		// Mock returns [2,0,1]: c,a,b. The original order is a,b,c.
		let llm: LlmFunc = Arc::new(|_p: &str| "[2,0,1]".to_string());
		let mut results = vec![
			scored("a", "alpha", 0.9),
			scored("b", "beta", 0.8),
			scored("c", "gamma", 0.7),
		];
		llm_rerank(&cfg(3), Some(&llm), "q", &mut results);
		assert_eq!(
			ids(&results),
			vec!["c", "a", "b"],
			"head reordered to the LLM ranking"
		);
	}

	#[test]
	fn llm_rerank_keeps_tail_beyond_pool_in_place() {
		// pool=2: only a,b are reranked; c,d keep their original positions after.
		let llm: LlmFunc = Arc::new(|_p: &str| "[1,0]".to_string());
		let mut results = vec![
			scored("a", "a", 0.9),
			scored("b", "b", 0.8),
			scored("c", "c", 0.7),
			scored("d", "d", 0.6),
		];
		llm_rerank(&cfg(2), Some(&llm), "q", &mut results);
		assert_eq!(
			ids(&results),
			vec!["b", "a", "c", "d"],
			"head swapped, tail untouched"
		);
	}

	#[test]
	fn llm_rerank_unparseable_response_leaves_order_untouched() {
		let llm: LlmFunc = Arc::new(|_p: &str| "sorry, no idea".to_string());
		let mut results = vec![
			scored("a", "a", 0.9),
			scored("b", "b", 0.8),
			scored("c", "c", 0.7),
		];
		llm_rerank(&cfg(3), Some(&llm), "q", &mut results);
		assert_eq!(
			ids(&results),
			vec!["a", "b", "c"],
			"garbage ranking is a no-op"
		);
	}

	#[test]
	fn llm_rerank_caps_candidates_in_prompt_regardless_of_pool_size() {
		// rerank_pool_size is huge and there are 40 results, but the prompt must
		// list at most MAX_RERANK_CANDIDATES candidates.
		let seen = Arc::new(Mutex::new(String::new()));
		let captured = seen.clone();
		let llm: LlmFunc = Arc::new(move |p: &str| {
			*captured.lock().unwrap() = p.to_string();
			"[0]".to_string()
		});
		let mut results: Vec<ScoredEntity> = (0..40)
			.map(|i| scored(&format!("e{i}"), "txt", 1.0 - i as f64 * 0.01))
			.collect();
		llm_rerank(&cfg(1000), Some(&llm), "q", &mut results);

		let prompt = seen.lock().unwrap().clone();
		let last = MAX_RERANK_CANDIDATES - 1;
		assert!(
			prompt.contains(&format!("[{last}]")),
			"includes the last allowed candidate"
		);
		assert!(
			!prompt.contains(&format!("[{MAX_RERANK_CANDIDATES}]")),
			"never lists a candidate at the cap index"
		);
	}

	#[test]
	fn llm_rerank_disabled_or_no_llm_is_a_noop() {
		let llm: LlmFunc = Arc::new(|_p: &str| "[1,0]".to_string());
		let mut results = vec![scored("a", "a", 0.9), scored("b", "b", 0.8)];
		// Disabled in config.
		let mut disabled = cfg(2);
		disabled.rerank_enabled = false;
		llm_rerank(&disabled, Some(&llm), "q", &mut results);
		assert_eq!(ids(&results), vec!["a", "b"]);
		// No llm handle.
		llm_rerank(&cfg(2), None, "q", &mut results);
		assert_eq!(ids(&results), vec!["a", "b"]);
	}

	#[test]
	fn parses_clean_array() {
		assert_eq!(parse_ranking("[2,0,1]", 3), Some(vec![2, 0, 1]));
	}

	#[test]
	fn tolerates_surrounding_prose() {
		assert_eq!(parse_ranking("Ranking: [1,0] done", 2), Some(vec![1, 0]));
	}

	#[test]
	fn filters_out_of_range_indices() {
		// 5 >= pool(2) is dropped; 0 kept.
		assert_eq!(parse_ranking("[5,0]", 2), Some(vec![0]));
	}

	#[test]
	fn negative_index_is_filtered_not_panic() {
		// -1 as usize is huge -> filtered by the `< pool` check, no panic.
		assert_eq!(parse_ranking("[-1,1]", 2), Some(vec![1]));
	}

	#[test]
	fn no_brackets_is_none() {
		assert_eq!(parse_ranking("no ranking here", 3), None);
	}

	#[test]
	fn empty_array_is_none() {
		assert_eq!(parse_ranking("[]", 3), None);
	}

	#[test]
	fn whole_number_floats_accepted() {
		// Some LLMs emit [1.0, 0.0, 2.0] instead of [1, 0, 2] — still valid.
		assert_eq!(parse_ranking("[1.0, 0.0, 2.0]", 3), Some(vec![1, 0, 2]));
	}

	#[test]
	fn fractional_float_discards_ranking() {
		// 1.5 is not a valid index — bail the whole ranking (don't trust partial).
		assert_eq!(parse_ranking("[1.5, 0]", 3), None);
	}
}
