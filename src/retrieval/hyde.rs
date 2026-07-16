use crate::config::RetrievalConfig;
use crate::retrieval::{EmbedFunc, LlmFunc};

pub fn expand_query(
	cfg: &RetrievalConfig,
	llm: Option<&LlmFunc>,
	embed: Option<&EmbedFunc>,
	query_vec: &[f32],
	query_text: &str,
) -> Vec<f32> {
	if !cfg.hyde_enabled {
		return query_vec.to_vec();
	}
	let tokens = query_text.split_whitespace().count();
	if tokens == 0 || tokens >= cfg.hyde_min_query_tokens {
		return query_vec.to_vec();
	}
	let (llm_fn, embed_fn) = match (llm, embed) {
		(Some(l), Some(e)) => (l, e),
		_ => return query_vec.to_vec(),
	};

	let prompt = format!("Write one paragraph that would answer: {query_text}");
	let hypo = llm_fn(&prompt);
	if hypo.trim().is_empty() {
		return query_vec.to_vec();
	}
	let hypo_vec = match embed_fn(&hypo) {
		Ok(v) => v,
		Err(_) => return query_vec.to_vec(),
	};
	if hypo_vec.len() != query_vec.len() || hypo_vec.is_empty() {
		return query_vec.to_vec();
	}

	let w = cfg.hyde_fusion_weight as f32;
	let mut fused: Vec<f32> = query_vec
		.iter()
		.zip(hypo_vec.iter())
		.map(|(q, h)| q * (1.0 - w) + h * w)
		.collect();
	crate::base::math::l2_normalize(&mut fused);
	fused
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::Arc;

	#[test]
	fn disabled_returns_query_unchanged() {
		let cfg = RetrievalConfig {
			hyde_enabled: false,
			..Default::default()
		};
		assert_eq!(
			expand_query(&cfg, None, None, &[1.0, 2.0], "cat"),
			vec![1.0, 2.0]
		);
	}

	#[test]
	fn long_query_skips_expansion() {
		let cfg = RetrievalConfig::default(); // hyde_min_query_tokens = 6
		let llm: LlmFunc = Arc::new(|_: &str| "x".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![9.0, 9.0]));
		let qv = vec![1.0, 0.0];
		let out = expand_query(
			&cfg,
			Some(&llm),
			Some(&embed),
			&qv,
			"one two three four five six",
		);
		assert_eq!(out, qv, "queries at/over the token floor are not expanded");
	}

	#[test]
	fn missing_llm_or_embed_returns_query() {
		let cfg = RetrievalConfig::default();
		assert_eq!(
			expand_query(&cfg, None, None, &[1.0, 0.0], "cat"),
			vec![1.0, 0.0]
		);
	}

	#[test]
	fn short_query_fuses_and_normalizes() {
		let cfg = RetrievalConfig::default();
		let llm: LlmFunc = Arc::new(|_: &str| "a hypothetical answer".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![0.0, 1.0]));
		let qv = vec![1.0, 0.0];
		let out = expand_query(&cfg, Some(&llm), Some(&embed), &qv, "cat");
		// fused (0.5,0.5) -> L2-normalized: equal components, unit norm.
		assert!((out[0] - out[1]).abs() < 1e-6);
		let norm = out.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>().sqrt();
		assert!((norm - 1.0).abs() < 1e-6, "fused vector is L2-normalized");
	}

	#[test]
	fn fusion_weight_one_yields_pure_hypo_direction() {
		// w=1.0 → fused = hypo (then L2-normalized): drops the query component.
		let cfg = RetrievalConfig {
			hyde_fusion_weight: 1.0,
			..Default::default()
		};
		let llm: LlmFunc = Arc::new(|_: &str| "answer".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![0.0, 3.0]));
		let qv = vec![1.0, 0.0];
		let out = expand_query(&cfg, Some(&llm), Some(&embed), &qv, "cat");
		// pure hypo (0,3) normalized → (0,1).
		assert!(out[0].abs() < 1e-9 && (out[1] - 1.0).abs() < 1e-9);
	}

	#[test]
	fn empty_hypothesis_returns_query() {
		let cfg = RetrievalConfig::default();
		let llm: LlmFunc = Arc::new(|_: &str| "   ".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![0.0, 1.0]));
		let qv = vec![1.0, 0.0];
		assert_eq!(expand_query(&cfg, Some(&llm), Some(&embed), &qv, "cat"), qv);
	}

	#[test]
	fn embed_length_mismatch_returns_query() {
		let cfg = RetrievalConfig::default();
		let llm: LlmFunc = Arc::new(|_: &str| "answer".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![1.0, 2.0, 3.0])); // len 3
		let qv = vec![1.0, 0.0]; // len 2
		assert_eq!(expand_query(&cfg, Some(&llm), Some(&embed), &qv, "cat"), qv);
	}

	#[test]
	fn empty_hypo_vec_returns_query_without_fusing() {
		// Dimension-0 edge: embed returns Ok(vec![]). The len/empty guard must
		// short-circuit to the original query rather than fuse against a 0-d
		// vector (which would zip to nothing and yield an empty result).
		let cfg = RetrievalConfig::default();
		let llm: LlmFunc = Arc::new(|_: &str| "answer".to_string());
		let embed: EmbedFunc = Arc::new(|_: &str| Ok(vec![])); // 0-dim
		let qv = vec![1.0, 0.0];
		assert_eq!(expand_query(&cfg, Some(&llm), Some(&embed), &qv, "cat"), qv);
	}
}
