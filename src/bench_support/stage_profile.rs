//! Per-stage profiling leg for the retrieval harness — the stage-level companion
//! to [`latency`](super::latency)'s whole-path percentiles. Runs the (LLM-free)
//! graph phase through [`retrieve_profiled`](crate::retrieval::answer::retrieve_profiled)
//! and aggregates each stage's mean/p50/p95 (ms) plus its share of the total, so a
//! config/index change can be attributed to the stage it moved.

use std::collections::BTreeMap;

use crate::base::graph::GraphGnn;
use crate::config::RetrievalConfig;
use crate::retrieval::seed::{Mode, Weights};

use super::embed;
use super::trace::Trace;
use crate::base::util::percentile_sorted;

struct StageStats {
	label: String,
	mean_ms: f64,
	p50_ms: f64,
	p95_ms: f64,
	share: f64,
}

pub struct StageProfileReport {
	pub trace_name: String,
	pub samples: usize,
	total_ms: f64,
	stages: Vec<StageStats>,
}

/// Run every query `iters` times (after `warmup` untimed passes) through the
/// profiled graph phase, collecting each stage's per-run timing. Stages are keyed
/// by label and aggregated across all queries and iterations. The ordering of the
/// output follows first-seen stage order so the table reads seed → chains.
pub fn measure_stage_profile(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	trace: &Trace,
	warmup: usize,
	iters: usize,
) -> StageProfileReport {
	let mut order: Vec<String> = Vec::new();
	let mut samples: BTreeMap<String, Vec<f64>> = BTreeMap::new();
	let mut totals: Vec<f64> = Vec::with_capacity(trace.queries.len() * iters.max(1));

	for q in &trace.queries {
		let mode = Mode::parse(&q.mode);
		let qvec = embed::embed(&q.query);
		let opts = q
			.filter_kind
			.as_deref()
			.and_then(crate::base::types::EntityKind::parse)
			.map(|kind| crate::retrieval::score::QueryOptions {
				kind: Some(kind),
				..Default::default()
			});
		let w = Weights::for_mode(cfg, mode);

		for _ in 0..warmup {
			let _ = crate::retrieval::answer::retrieve_profiled(
				g,
				cfg,
				&qvec,
				&q.query,
				mode,
				opts.as_ref(),
				w,
			);
		}
		for _ in 0..iters {
			let (_, prof) = crate::retrieval::answer::retrieve_profiled(
				g,
				cfg,
				&qvec,
				&q.query,
				mode,
				opts.as_ref(),
				w,
			);
			totals.push(prof.total_ms);
			for c in &prof.checkpoints {
				if !samples.contains_key(&c.label) {
					order.push(c.label.clone());
				}
				samples.entry(c.label.clone()).or_default().push(c.elapsed_ms);
			}
		}
	}

	let n = totals.len();
	let total_ms = if n == 0 {
		0.0
	} else {
		totals.iter().sum::<f64>() / n as f64
	};

	let stages = order
		.into_iter()
		.map(|label| {
			let mut xs = samples.remove(&label).unwrap_or_default();
			let mean = if xs.is_empty() {
				0.0
			} else {
				xs.iter().sum::<f64>() / xs.len() as f64
			};
			xs.sort_by(crate::base::util::cmp_partial);
			StageStats {
				label,
				mean_ms: mean,
				p50_ms: percentile_sorted(&xs, 0.50).unwrap_or(0.0),
				p95_ms: percentile_sorted(&xs, 0.95).unwrap_or(0.0),
				share: if total_ms > 0.0 { mean / total_ms } else { 0.0 },
			}
		})
		.collect();

	StageProfileReport {
		trace_name: trace.name.clone(),
		samples: n,
		total_ms,
		stages,
	}
}

impl std::fmt::Display for StageProfileReport {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		writeln!(
			f,
			"trace: {}   samples: {}   mean total: {:.3}ms",
			self.trace_name, self.samples, self.total_ms
		)?;
		writeln!(
			f,
			"{:<16} {:>10} {:>10} {:>10} {:>8}",
			"stage", "mean(ms)", "p50(ms)", "p95(ms)", "share"
		)?;
		for s in &self.stages {
			writeln!(
				f,
				"{:<16} {:>10.3} {:>10.3} {:>10.3} {:>7.1}%",
				s.label,
				s.mean_ms,
				s.p50_ms,
				s.p95_ms,
				s.share * 100.0
			)?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bench_support::build::build_graph;
	use crate::bench_support::trace::{TraceDoc, TraceQuery};

	fn tiny_trace() -> Trace {
		Trace {
			name: "stage".into(),
			docs: vec![
				TraceDoc {
					id: "d1".into(),
					text: "rust ownership borrow checker".into(),
					kind: None,
				},
				TraceDoc {
					id: "d2".into(),
					text: "graph neural network".into(),
					kind: None,
				},
			],
			queries: vec![TraceQuery {
				id: "q1".into(),
				query: "rust ownership".into(),
				expected_ids: vec!["d1".into()],
				mode: "hybrid".into(),
				filter_kind: None,
			}],
		}
	}

	#[test]
	fn aggregates_stages_and_shares_sum_to_the_whole() {
		let trace = tiny_trace();
		let g = build_graph(&trace);
		let r = measure_stage_profile(&g, &RetrievalConfig::default(), &trace, 1, 5);
		assert_eq!(r.samples, 5, "1 query x 5 iters = 5 timed runs");
		assert!(!r.stages.is_empty(), "at least one stage recorded");
		assert!(
			r.stages.iter().all(|s| s.p50_ms <= s.p95_ms),
			"per-stage percentiles are monotonic"
		);
		let share_sum: f64 = r.stages.iter().map(|s| s.share).sum();
		// Stage means sum to roughly the mean total (checkpoint gaps == total), so
		// shares sum to ~1.0 barring the tiny inter-stage slack the Profiler leaves.
		assert!(
			(0.5..=1.5).contains(&share_sum),
			"stage shares sum near 1.0, got {share_sum}"
		);
	}

	#[test]
	fn renders_a_table_with_a_row_per_stage() {
		let trace = tiny_trace();
		let g = build_graph(&trace);
		let r = measure_stage_profile(&g, &RetrievalConfig::default(), &trace, 0, 2);
		let out = r.to_string();
		assert!(out.contains("stage"), "header present: {out}");
		assert!(out.contains("seed_dense"), "seed stage listed: {out}");
	}
}
