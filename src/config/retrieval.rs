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
			heat_half_life_secs: HeatConfig::defaults().half_life_secs,
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
		}
	}
}
