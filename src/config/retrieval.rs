use serde::{Deserialize, Serialize};

use crate::base::constants;
use crate::base::heat::HeatConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeWeights {
	pub content: f64,
	pub reason: f64,
	pub edge: f64,
}

impl Default for ModeWeights {
	fn default() -> Self {
		Self {
			content: constants::DEFAULT_WEIGHT_CONTENT,
			reason: constants::DEFAULT_WEIGHT_REASON,
			edge: constants::DEFAULT_WEIGHT_EDGE,
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
	pub gravity_weight: f64,
	// Multiplier on the final score of an entity held in a `remote-*` phantom kern.
	// Federation is unauthenticated: this is what stops peer-supplied content from
	// outranking local knowledge. 1.0 disables the penalty; 0.0 keeps remote
	// entities retrievable but always last.
	pub remote_trust_weight: f64,
	pub min_deliver_score: f64,
	pub max_deliver_results: usize,
	// Facts placed in the answer prompt. Retrieval delivers up to
	// `max_deliver_results`, so anything below that silently discards evidence
	// kern already found and ranked.
	pub answer_max_facts: usize,
	// Tell the answerer to decline when the context looks insufficient.
	// Measured 2026-07-20: combined with a small `answer_max_facts` this made the
	// model abstain on 69% of ANSWERABLE probes — a starved prompt reads as
	// "the fact does not exist". Off until an A/B shows it earns its place.
	pub answer_abstain_hint: bool,
	pub important_min_cosine: f64,
	pub important_access_threshold: i32,
	pub weights_content: ModeWeights,
	pub weights_reason: ModeWeights,
	pub weights_hybrid: ModeWeights,
	pub rrf_k: f64,
	pub rrf_global_weight: f64,
	pub dedup_by_section: bool,
	pub mmr_enabled: bool,
	pub mmr_lambda: f64,
	pub mmr_pool_size: usize,
	pub rerank_enabled: bool,
	pub rerank_pool_size: usize,
	pub hyde_enabled: bool,
	pub hyde_min_query_tokens: usize,
	pub hyde_fusion_weight: f64,
	pub lexical_enabled: bool,
	pub bm25_k1: f64,
	pub bm25_b: f64,
	pub pagerank_enabled: bool,
	pub pagerank_damping: f64,
	pub pagerank_iters: usize,
	pub pagerank_top_k: usize,
	pub query_cache_cap: usize,
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
			gravity_weight: 0.15,
			remote_trust_weight: 0.4,
			min_deliver_score: 0.0,
			max_deliver_results: 25,
			answer_max_facts: constants::ANSWER_MAX_THOUGHTS,
			answer_abstain_hint: false,
			important_min_cosine: constants::IMPORTANT_MIN_COSINE,
			important_access_threshold: constants::IMPORTANT_ACCESS_THRESHOLD,
			weights_content: ModeWeights {
				content: 0.70,
				reason: 0.15,
				edge: 0.15,
			},
			weights_reason: ModeWeights {
				content: 0.20,
				reason: 0.60,
				edge: 0.20,
			},
			weights_hybrid: ModeWeights::default(),
			rrf_k: 60.0,
			rrf_global_weight: 0.5,
			dedup_by_section: true,
			mmr_enabled: true,
			mmr_lambda: 0.75,
			mmr_pool_size: 50,
			rerank_enabled: true,
			rerank_pool_size: 30,
			hyde_enabled: true,
			hyde_min_query_tokens: 6,
			hyde_fusion_weight: 0.5,
			lexical_enabled: true,
			bm25_k1: 1.2,
			bm25_b: 0.75,
			pagerank_enabled: true,
			pagerank_damping: 0.85,
			pagerank_iters: 25,
			pagerank_top_k: 100,
			query_cache_cap: constants::QUERY_CACHE_DEFAULT_CAP,
			query_cache_theta: constants::QUERY_CACHE_DEFAULT_THETA,
		}
	}
}

impl RetrievalConfig {
	pub fn validate(&self) -> Vec<String> {
		let mut errs = Vec::new();

		for (name, w) in [
			("content", &self.weights_content),
			("reason", &self.weights_reason),
			("hybrid", &self.weights_hybrid),
		] {
			let sum = w.content + w.reason + w.edge;
			if (sum - 1.0).abs() > 0.01 {
				errs.push(format!("weights_{name} sum to {sum:.3}, expected ~1.0"));
			}
		}

		for (name, v) in [
			("query_cache_theta", self.query_cache_theta),
			("mmr_lambda", self.mmr_lambda),
			("hyde_fusion_weight", self.hyde_fusion_weight),
			("bm25_b", self.bm25_b),
			("remote_trust_weight", self.remote_trust_weight),
		] {
			if !(0.0..=1.0).contains(&v) {
				errs.push(format!("{name} ({v}) must be in [0.0, 1.0]"));
			}
		}

		if self.bm25_k1 < 0.0 {
			errs.push(format!("bm25_k1 ({}) must be >= 0.0", self.bm25_k1));
		}

		if !(0.0..1.0).contains(&self.pagerank_damping) {
			errs.push(format!(
				"pagerank_damping ({}) must be in [0.0, 1.0)",
				self.pagerank_damping
			));
		}

		if self.gravity_weight < 0.0 {
			errs.push(format!(
				"gravity_weight ({}) must be >= 0.0",
				self.gravity_weight
			));
		}
		if self.rrf_k < 0.0 {
			errs.push(format!("rrf_k ({}) must be >= 0.0", self.rrf_k));
		}
		if self.seed_k == 0 {
			errs.push("seed_k must be >= 1 (0 seeds nothing, so every query is empty)".to_string());
		}
		if self.max_deliver_results == 0 {
			errs.push("max_deliver_results must be >= 1 (0 delivers nothing)".to_string());
		}
		if self.answer_max_facts == 0 {
			errs.push(
				"answer_max_facts must be >= 1 (0 puts no evidence in the prompt, so every answer abstains)"
					.to_string(),
			);
		}
		if self.answer_max_facts > self.max_deliver_results {
			errs.push(format!(
				"answer_max_facts ({}) exceeds max_deliver_results ({}) — retrieval never delivers that many, so the extra is dead",
				self.answer_max_facts, self.max_deliver_results,
			));
		}

		errs
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_config_is_valid() {
		assert!(
			RetrievalConfig::default().validate().is_empty(),
			"shipped defaults must validate"
		);
	}

	#[test]
	fn weights_not_summing_to_one_are_flagged() {
		let mut cfg = RetrievalConfig::default();
		cfg.weights_hybrid.content = 0.9;
		let errs = cfg.validate();
		assert!(
			errs.iter().any(|e| e.contains("weights_hybrid")),
			"got {errs:?}"
		);
	}

	#[test]
	fn out_of_range_unit_interval_fields_are_flagged() {
		let cfg = RetrievalConfig {
			query_cache_theta: 1.5,
			mmr_lambda: -0.1,
			..Default::default()
		};
		let errs = cfg.validate();
		assert!(
			errs.iter().any(|e| e.contains("query_cache_theta")),
			"got {errs:?}"
		);
		assert!(
			errs.iter().any(|e| e.contains("mmr_lambda")),
			"got {errs:?}"
		);
	}

	#[test]
	fn out_of_range_bm25_params_are_flagged() {
		let bad_b = RetrievalConfig {
			bm25_b: 2.0,
			..Default::default()
		};
		assert!(
			bad_b.validate().iter().any(|e| e.contains("bm25_b")),
			"bm25_b > 1"
		);

		let neg_k1 = RetrievalConfig {
			bm25_k1: -0.5,
			..Default::default()
		};
		assert!(
			neg_k1.validate().iter().any(|e| e.contains("bm25_k1")),
			"negative bm25_k1"
		);
	}

	#[test]
	fn answer_fact_budget_is_bounded_by_what_retrieval_delivers() {
		let none = RetrievalConfig {
			answer_max_facts: 0,
			..Default::default()
		};
		assert!(
			none
				.validate()
				.iter()
				.any(|e| e.contains("answer_max_facts")),
			"0 facts means every answer abstains"
		);

		let over = RetrievalConfig {
			answer_max_facts: 40,
			max_deliver_results: 25,
			..Default::default()
		};
		assert!(
			over.validate().iter().any(|e| e.contains("dead")),
			"asking for more facts than retrieval delivers is a config error"
		);

		let ok = RetrievalConfig {
			answer_max_facts: 25,
			max_deliver_results: 25,
			..Default::default()
		};
		assert!(
			!ok.validate().iter().any(|e| e.contains("answer_max_facts")),
			"using the full delivered set is valid"
		);
	}

	#[test]
	fn retrieval_breaking_values_are_flagged() {
		let neg_rrf = RetrievalConfig {
			rrf_k: -1.0,
			..Default::default()
		};
		assert!(
			neg_rrf.validate().iter().any(|e| e.contains("rrf_k")),
			"negative rrf_k"
		);

		let zero_seed = RetrievalConfig {
			seed_k: 0,
			..Default::default()
		};
		assert!(
			zero_seed.validate().iter().any(|e| e.contains("seed_k")),
			"seed_k 0"
		);

		let zero_deliver = RetrievalConfig {
			max_deliver_results: 0,
			..Default::default()
		};
		assert!(
			zero_deliver
				.validate()
				.iter()
				.any(|e| e.contains("max_deliver_results")),
			"max_deliver_results 0"
		);

		let neg_gravity = RetrievalConfig {
			gravity_weight: -0.1,
			..Default::default()
		};
		assert!(
			neg_gravity
				.validate()
				.iter()
				.any(|e| e.contains("gravity_weight")),
			"negative gravity_weight"
		);

		let zero_rrf = RetrievalConfig {
			rrf_k: 0.0,
			..Default::default()
		};
		assert!(
			!zero_rrf.validate().iter().any(|e| e.contains("rrf_k")),
			"rrf_k 0 is valid, must not flag"
		);
	}
}
