use serde::{Deserialize, Serialize};

use crate::base::constants;
use crate::base::heat::HeatConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeWeights {
	pub content: f64,
	pub reason: f64,
	pub edge: f64,
	pub lexical: f64,
}

impl Default for ModeWeights {
	fn default() -> Self {
		Self {
			content: 0.50,
			reason: 0.30,
			edge: 0.20,
			lexical: 0.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalConfig {
	pub seed_k: usize,
	pub max_expansions: usize,
	pub decay: f64,
	pub qbst_access_weight: f64,
	pub qbst_recency_weight: f64,
	pub qbst_recency_half_life_secs: u64,
	pub qbst_cap: f64,
	pub heat_half_life_secs: u64,
	pub refine_traversal_weight: f64,
	pub refine_boost_cap: f64,
	pub fact_score_boost: f64,
	pub min_deliver_score: f64,
	pub max_deliver_results: usize,
	pub important_min_cosine: f64,
	pub important_access_threshold: i32,
	pub weights_content: ModeWeights,
	pub weights_reason: ModeWeights,
	pub weights_hybrid: ModeWeights,
	pub rrf_k: f64,
	/// Weighted-RRF multiplier for the query-INDEPENDENT seed lists (global
	/// importance + PageRank) relative to the query-relevant dense/lexical lists
	/// (which stay 1.0). `< 1.0` down-weights global priors so a
	/// popular-but-irrelevant entity can't match a query-relevant hit; `1.0`
	/// recovers plain unweighted RRF.
	pub rrf_global_weight: f64,
	pub dedup_by_section: bool,
	pub mmr_enabled: bool,
	pub mmr_lambda: f64,
	pub mmr_pool_size: usize,
	pub rerank_enabled: bool,
	pub rerank_pool_size: usize,
	pub hyde_enabled: bool,
	pub hyde_min_query_tokens: usize,
	/// Weight on the hypothetical-document vector when HyDE fuses it with the
	/// raw query vector: `fused = query*(1-w) + hypo*w`, then L2-normalized.
	/// `0.5` is the symmetric blend (original behavior); higher trusts the
	/// generated hypo more, lower stays closer to the literal query.
	pub hyde_fusion_weight: f64,
	pub lexical_enabled: bool,
	pub bm25_k1: f64,
	pub bm25_b: f64,
	pub softmax_temperature: f64,
	pub pagerank_enabled: bool,
	pub pagerank_damping: f64,
	pub pagerank_iters: usize,
	pub pagerank_top_k: usize,
	pub adaptive_ef_enabled: bool,
	pub adaptive_ef_start: usize,
	pub adaptive_ef_max: usize,
	pub adaptive_ef_step: usize,
	pub adaptive_ef_spread_epsilon: f64,
	/// Semantic query cache: number of answered queries retained before LRU
	/// eviction. `0` disables the cache.
	pub query_cache_cap: usize,
	/// Cosine floor for a semantic cache hit. High (≈0.97) so only paraphrases
	/// and re-asks share an entry, never merely topical neighbours.
	pub query_cache_theta: f64,
}

impl Default for RetrievalConfig {
	fn default() -> Self {
		Self {
			seed_k: 15,
			max_expansions: 500,
			decay: 0.25,
			qbst_access_weight: constants::QBST_ACCESS_WEIGHT,
			qbst_recency_weight: constants::QBST_RECENCY_WEIGHT,
			qbst_recency_half_life_secs: constants::QBST_RECENCY_HALF_LIFE.as_secs(),
			qbst_cap: constants::QBST_CAP,
			heat_half_life_secs: HeatConfig::default().half_life_secs,
			refine_traversal_weight: constants::REFINE_TRAVERSAL_WEIGHT,
			refine_boost_cap: constants::REFINE_BOOST_CAP,
			fact_score_boost: constants::FACT_SCORE_BOOST,
			min_deliver_score: 0.0,
			max_deliver_results: 25,
			important_min_cosine: constants::IMPORTANT_MIN_COSINE,
			important_access_threshold: constants::IMPORTANT_ACCESS_THRESHOLD,
			weights_content: ModeWeights {
				content: 0.70,
				reason: 0.15,
				edge: 0.15,
				lexical: 0.0,
			},
			weights_reason: ModeWeights {
				content: 0.20,
				reason: 0.60,
				edge: 0.20,
				lexical: 0.0,
			},
			weights_hybrid: ModeWeights {
				content: 0.50,
				reason: 0.30,
				edge: 0.20,
				lexical: 0.0,
			},
			rrf_k: 60.0,
			rrf_global_weight: 0.5,
			dedup_by_section: true,
			mmr_enabled: true,
			mmr_lambda: 0.45,
			mmr_pool_size: 50,
			rerank_enabled: true,
			rerank_pool_size: 30,
			hyde_enabled: true,
			hyde_min_query_tokens: 6,
			hyde_fusion_weight: 0.5,
			lexical_enabled: true,
			bm25_k1: 1.2,
			bm25_b: 0.75,
			softmax_temperature: 1.0,
			pagerank_enabled: true,
			pagerank_damping: 0.85,
			pagerank_iters: 25,
			pagerank_top_k: 100,
			adaptive_ef_enabled: false,
			adaptive_ef_start: 16,
			adaptive_ef_max: 128,
			adaptive_ef_step: 128,
			adaptive_ef_spread_epsilon: 0.02,
			query_cache_cap: constants::QUERY_CACHE_DEFAULT_CAP,
			query_cache_theta: constants::QUERY_CACHE_DEFAULT_THETA,
		}
	}
}

impl RetrievalConfig {
	/// Check cross-field invariants on a loaded config. Returns a list of
	/// human-readable problems (empty = valid) so a caller can warn-and-continue or
	/// reject as it sees fit. Cheap structural sanity check, not a tuning oracle.
	pub fn validate(&self) -> Vec<String> {
		let mut errs = Vec::new();

		for (name, w) in [
			("content", &self.weights_content),
			("reason", &self.weights_reason),
			("hybrid", &self.weights_hybrid),
		] {
			let sum = w.content + w.reason + w.edge + w.lexical;
			if (sum - 1.0).abs() > 0.01 {
				errs.push(format!("weights_{name} sum to {sum:.3}, expected ~1.0"));
			}
		}

		if self.adaptive_ef_start > self.adaptive_ef_max {
			errs.push(format!(
				"adaptive_ef_start ({}) must be <= adaptive_ef_max ({})",
				self.adaptive_ef_start, self.adaptive_ef_max,
			));
		}

		for (name, v) in [
			("query_cache_theta", self.query_cache_theta),
			("mmr_lambda", self.mmr_lambda),
			("hyde_fusion_weight", self.hyde_fusion_weight),
		] {
			if !(0.0..=1.0).contains(&v) {
				errs.push(format!("{name} ({v}) must be in [0.0, 1.0]"));
			}
		}

		if !(0.0..1.0).contains(&self.pagerank_damping) {
			errs.push(format!("pagerank_damping ({}) must be in [0.0, 1.0)", self.pagerank_damping));
		}

		errs
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_config_is_valid() {
		assert!(RetrievalConfig::default().validate().is_empty(), "shipped defaults must validate");
	}

	#[test]
	fn weights_not_summing_to_one_are_flagged() {
		let mut cfg = RetrievalConfig::default();
		cfg.weights_hybrid.content = 0.9; // 0.9 + 0.30 + 0.20 + 0.0 = 1.4
		let errs = cfg.validate();
		assert!(errs.iter().any(|e| e.contains("weights_hybrid")), "got {errs:?}");
	}

	#[test]
	fn adaptive_ef_start_above_max_is_flagged() {
		let mut cfg = RetrievalConfig::default();
		cfg.adaptive_ef_start = 200;
		cfg.adaptive_ef_max = 128;
		assert!(cfg.validate().iter().any(|e| e.contains("adaptive_ef_start")));
	}

	#[test]
	fn out_of_range_unit_interval_fields_are_flagged() {
		let mut cfg = RetrievalConfig::default();
		cfg.query_cache_theta = 1.5;
		cfg.mmr_lambda = -0.1;
		let errs = cfg.validate();
		assert!(errs.iter().any(|e| e.contains("query_cache_theta")), "got {errs:?}");
		assert!(errs.iter().any(|e| e.contains("mmr_lambda")), "got {errs:?}");
	}
}
