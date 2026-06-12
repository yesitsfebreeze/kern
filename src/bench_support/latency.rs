//! Performance measurement for the trace harness — the speed complement to
//! [`replay`](super::replay)'s recall/NDCG quality scoring. Two views, both over
//! the (LLM-free) graph retrieval path so index/config changes can be A/B'd:
//!
//! - [`measure_latency`] — single-reader p50/p95/p99 + mean over warmup+timed
//!   iterations of each query.
//! - [`measure_throughput`] — queries/sec with `threads` concurrent readers, which
//!   exercises the read-only graph's concurrent-read scaling (the path the MCP
//!   server and recall hooks share).
//!
//! These are kern-internal A/B numbers over a FIXED trace ("did a change move
//! latency / throughput?"), not absolute SLAs and not yet a Qdrant baseline.

use std::time::Instant;

use crate::base::graph::GraphGnn;
use crate::config::RetrievalConfig;
use crate::retrieval::seed::Mode;

use super::embed;
use super::trace::Trace;

#[derive(Debug, Clone)]
pub struct LatencyReport {
	pub trace_name: String,
	pub samples: usize,
	pub mean_ms: f64,
	pub p50_ms: f64,
	pub p95_ms: f64,
	pub p99_ms: f64,
}

/// Nearest-rank percentile of an ascending-SORTED slice. `p` in `[0, 1]`. An
/// empty slice is `0.0`; `p <= 0` returns the first element, `p >= 1` the last.
fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
	if sorted.is_empty() {
		return 0.0;
	}
	if p <= 0.0 {
		return sorted[0];
	}
	if p >= 1.0 {
		return sorted[sorted.len() - 1];
	}
	// Nearest-rank: 1-based rank = ceil(p * n), clamped into range.
	let rank = (p * sorted.len() as f64).ceil() as usize;
	sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Time the retrieval path for every query in `trace`. The LLM/embedder hooks are
/// `None`, so this measures only the graph/index work (the sub-ms path), never an
/// LLM leg. The same `filter_kind` the recall harness uses is applied, so a
/// filtered run measures the filtered traversal's cost.
pub fn measure_latency(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	trace: &Trace,
	warmup: usize,
	iters: usize,
) -> LatencyReport {
	let mut timings: Vec<f64> = Vec::with_capacity(trace.queries.len() * iters.max(1));
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
		for _ in 0..warmup {
			let _ = crate::retrieval::answer::query(g, cfg, &qvec, &q.query, mode, None, None, opts.clone());
		}
		for _ in 0..iters {
			let t0 = Instant::now();
			let _ = crate::retrieval::answer::query(g, cfg, &qvec, &q.query, mode, None, None, opts.clone());
			timings.push(t0.elapsed().as_secs_f64() * 1000.0);
		}
	}

	let samples = timings.len();
	let mean_ms = if samples == 0 {
		0.0
	} else {
		timings.iter().sum::<f64>() / samples as f64
	};
	timings.sort_by(crate::base::util::cmp_partial);
	LatencyReport {
		trace_name: trace.name.clone(),
		samples,
		mean_ms,
		p50_ms: percentile_sorted(&timings, 0.50),
		p95_ms: percentile_sorted(&timings, 0.95),
		p99_ms: percentile_sorted(&timings, 0.99),
	}
}

#[derive(Debug, Clone)]
pub struct ThroughputReport {
	pub trace_name: String,
	pub threads: usize,
	/// Total queries executed across all threads.
	pub total_queries: usize,
	pub elapsed_secs: f64,
	pub qps: f64,
}

/// Run the whole trace `per_thread_iters` times on each of `threads` concurrent
/// readers and report queries/sec. The graph is shared `&GraphGnn` across scoped
/// threads — retrieval never mutates it, so this measures honest concurrent-read
/// scaling (a `RwLock` write would serialize; there is none on this path).
pub fn measure_throughput(
	g: &GraphGnn,
	cfg: &RetrievalConfig,
	trace: &Trace,
	threads: usize,
	per_thread_iters: usize,
) -> ThroughputReport {
	let threads = threads.max(1);
	let start = Instant::now();
	std::thread::scope(|s| {
		for _ in 0..threads {
			s.spawn(|| {
				for _ in 0..per_thread_iters {
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
						let _ = crate::retrieval::answer::query(g, cfg, &qvec, &q.query, mode, None, None, opts);
					}
				}
			});
		}
	});
	let elapsed_secs = start.elapsed().as_secs_f64();
	let total_queries = threads * per_thread_iters * trace.queries.len();
	let qps = if elapsed_secs > 0.0 {
		total_queries as f64 / elapsed_secs
	} else {
		0.0
	};
	ThroughputReport {
		trace_name: trace.name.clone(),
		threads,
		total_queries,
		elapsed_secs,
		qps,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bench_support::build::build_graph;
	use crate::bench_support::trace::{TraceDoc, TraceQuery};

	#[test]
	fn percentile_sorted_uses_nearest_rank_and_handles_edges() {
		let xs: Vec<f64> = (1..=10).map(|i| i as f64).collect(); // 1..=10, sorted
		assert_eq!(percentile_sorted(&xs, 0.0), 1.0, "p0 -> first");
		assert_eq!(percentile_sorted(&xs, 1.0), 10.0, "p100 -> last");
		// nearest-rank: ceil(0.5*10)=5 -> xs[4] = 5.0
		assert_eq!(percentile_sorted(&xs, 0.5), 5.0);
		// ceil(0.95*10)=10 -> xs[9] = 10.0; ceil(0.9*10)=9 -> xs[8]=9.0
		assert_eq!(percentile_sorted(&xs, 0.9), 9.0);
		assert_eq!(percentile_sorted(&xs, 0.95), 10.0);
		assert_eq!(percentile_sorted(&[], 0.5), 0.0, "empty -> 0");
	}

	#[test]
	fn measure_latency_pools_samples_and_orders_percentiles() {
		let trace = Trace {
			name: "lat".into(),
			docs: vec![
				TraceDoc { id: "d1".into(), text: "rust ownership borrow checker".into(), kind: None },
				TraceDoc { id: "d2".into(), text: "graph neural network".into(), kind: None },
			],
			queries: vec![TraceQuery {
				id: "q1".into(),
				query: "rust ownership".into(),
				expected_ids: vec!["d1".into()],
				mode: "hybrid".into(),
				filter_kind: None,
			}],
		};
		let g = build_graph(&trace);
		let r = measure_latency(&g, &RetrievalConfig::default(), &trace, 1, 5);
		assert_eq!(r.samples, 5, "1 query x 5 iters = 5 timed samples");
		assert!(r.mean_ms >= 0.0 && r.p50_ms >= 0.0);
		assert!(r.p50_ms <= r.p95_ms && r.p95_ms <= r.p99_ms, "percentiles are monotonic");
	}

	#[test]
	fn measure_throughput_runs_every_query_on_every_thread() {
		let trace = Trace {
			name: "tput".into(),
			docs: vec![
				TraceDoc { id: "d1".into(), text: "rust ownership borrow checker".into(), kind: None },
				TraceDoc { id: "d2".into(), text: "graph neural network".into(), kind: None },
			],
			queries: vec![
				TraceQuery { id: "q1".into(), query: "rust ownership".into(), expected_ids: vec!["d1".into()], mode: "hybrid".into(), filter_kind: None },
				TraceQuery { id: "q2".into(), query: "graph network".into(), expected_ids: vec!["d2".into()], mode: "hybrid".into(), filter_kind: None },
			],
		};
		let g = build_graph(&trace);
		let r = measure_throughput(&g, &RetrievalConfig::default(), &trace, 4, 3);
		assert_eq!(r.total_queries, 24, "4 threads x 3 iters x 2 queries");
		assert_eq!(r.threads, 4);
		assert!(r.qps > 0.0 && r.elapsed_secs >= 0.0, "positive throughput");
	}
}
