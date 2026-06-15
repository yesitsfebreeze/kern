use crate::base::graph::GraphGnn;
use crate::config::RetrievalConfig;

use super::replay::{replay, ReplayReport};
use super::trace::Trace;

#[derive(Debug, Clone)]
pub struct SweepRow {
	pub param: String,
	pub value: String,
	pub mean_ndcg10: f64,
	pub mean_recall10: f64,
	pub num_queries: usize,
}

/// A single tunable the bench harness can sweep over a list of values.
#[derive(Debug, Clone, Copy)]
pub enum SweepParam {
	/// Reciprocal-rank-fusion constant `k` in `1/(k + rank)`. Larger flattens the
	/// rank-weight curve so later ranks contribute relatively more. Typically 10–60.
	RrfK,
	/// Minimum blended score in `[0.0, 1.0]` a hit must clear to be delivered.
	/// Higher trims the tail (precision over recall).
	MinDeliverScore,
	/// MMR relevance-vs-diversity tradeoff in `[0.0, 1.0]`: 1.0 = pure relevance,
	/// 0.0 = pure diversity.
	MmrLambda,
	/// Number of seed entities pulled before graph expansion. Integer `>= 1`; a
	/// swept value `< 1` is clamped to 1 (with a warning) since 0 seeds nothing.
	SeedK,
}

impl SweepParam {
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"rrf_k" => Some(Self::RrfK),
			"min_deliver_score" => Some(Self::MinDeliverScore),
			"mmr_lambda" => Some(Self::MmrLambda),
			"seed_k" => Some(Self::SeedK),
			_ => None,
		}
	}

	pub fn name(&self) -> &'static str {
		match self {
			Self::RrfK => "rrf_k",
			Self::MinDeliverScore => "min_deliver_score",
			Self::MmrLambda => "mmr_lambda",
			Self::SeedK => "seed_k",
		}
	}
}

fn apply(cfg: &mut RetrievalConfig, param: SweepParam, value: f64) {
	match param {
		SweepParam::RrfK => cfg.rrf_k = value,
		SweepParam::MinDeliverScore => cfg.min_deliver_score = value,
		SweepParam::MmrLambda => cfg.mmr_lambda = value,
		SweepParam::SeedK => {
			if value < 1.0 {
				eprintln!("sweep: seed_k={value} is below 1 and was clamped to 1 (0 seeds nothing)");
			}
			cfg.seed_k = value.max(1.0) as usize;
		}
	}
}

pub fn sweep(g: &GraphGnn, trace: &Trace, param: SweepParam, values: &[f64]) -> Vec<SweepRow> {
	let mut rows = Vec::with_capacity(values.len());
	for &v in values {
		let mut cfg = RetrievalConfig::default();
		apply(&mut cfg, param, v);
		let report: ReplayReport = replay(g, &cfg, trace);
		rows.push(SweepRow {
			param: param.name().to_string(),
			value: format!("{v}"),
			mean_ndcg10: report.mean_ndcg10,
			mean_recall10: report.mean_recall10,
			num_queries: report.per_query.len(),
		});
	}
	rows
}

pub fn to_csv(rows: &[SweepRow]) -> String {
	let mut out = String::from("param,value,mean_ndcg10,mean_recall10,num_queries\n");
	for r in rows {
		out.push_str(&format!(
			"{},{},{:.6},{:.6},{}\n",
			r.param, r.value, r.mean_ndcg10, r.mean_recall10, r.num_queries
		));
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_round_trips_every_param() {
		for p in [
			SweepParam::RrfK,
			SweepParam::MinDeliverScore,
			SweepParam::MmrLambda,
			SweepParam::SeedK,
		] {
			let parsed = SweepParam::parse(p.name()).expect("name parses back");
			assert_eq!(parsed.name(), p.name(), "round-trip for {}", p.name());
		}
	}

	#[test]
	fn parse_unknown_is_none() {
		assert!(SweepParam::parse("bogus").is_none());
		assert!(SweepParam::parse("").is_none());
	}

	#[test]
	fn apply_mutates_only_the_targeted_field() {
		let mut cfg = RetrievalConfig::default();
		apply(&mut cfg, SweepParam::RrfK, 42.0);
		assert_eq!(cfg.rrf_k, 42.0);
		apply(&mut cfg, SweepParam::MinDeliverScore, 0.25);
		assert_eq!(cfg.min_deliver_score, 0.25);
		apply(&mut cfg, SweepParam::MmrLambda, 0.7);
		assert_eq!(cfg.mmr_lambda, 0.7);
		apply(&mut cfg, SweepParam::SeedK, 9.0);
		assert_eq!(cfg.seed_k, 9);
	}

	#[test]
	fn apply_clamps_seed_k_below_one_to_one() {
		let mut cfg = RetrievalConfig::default();
		apply(&mut cfg, SweepParam::SeedK, 0.0);
		assert_eq!(cfg.seed_k, 1, "a sub-1 seed_k floors at 1");
	}

	#[test]
	fn to_csv_has_a_header_and_one_six_decimal_row_per_entry() {
		let rows = vec![
			SweepRow {
				param: "rrf_k".into(),
				value: "10".into(),
				mean_ndcg10: 0.5,
				mean_recall10: 0.6,
				num_queries: 3,
			},
			SweepRow {
				param: "rrf_k".into(),
				value: "20".into(),
				mean_ndcg10: 0.75,
				mean_recall10: 0.8,
				num_queries: 3,
			},
		];
		let lines: Vec<String> = to_csv(&rows).lines().map(str::to_string).collect();
		assert_eq!(
			lines[0],
			"param,value,mean_ndcg10,mean_recall10,num_queries"
		);
		assert_eq!(lines.len(), 3, "header + 2 data rows");
		assert_eq!(
			lines[1], "rrf_k,10,0.500000,0.600000,3",
			"metrics formatted to 6 decimals"
		);
		assert_eq!(lines[2], "rrf_k,20,0.750000,0.800000,3");
	}
}
