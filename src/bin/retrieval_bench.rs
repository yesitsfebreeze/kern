//! `retrieval_bench` — replay a retrieval trace and report NDCG@10, optionally
//! sweeping one [`SweepParam`] over a list of values.
//!
//! The `--trace` file is JSON of the shape:
//! ```json
//! {
//!   "name": "my-trace",
//!   "docs":    [ { "id": "d1", "text": "..." } ],
//!   "queries": [ { "id": "q1", "query": "...", "expected_ids": ["d1"], "mode": "hybrid" } ]
//! }
//! ```
//! `docs` seed the graph; each `query` is scored against its `expected_ids` using
//! its declared `mode`. `--mode <m>` restricts the run to queries with that mode.

use clap::Parser;
use kern::bench_support::build::build_graph;
use kern::bench_support::replay::replay;
use kern::bench_support::sweep::{sweep, to_csv, SweepParam};
use kern::bench_support::trace;
use kern::config::RetrievalConfig;

#[derive(Parser, Debug)]
#[command(name = "retrieval_bench", about = "Replay retrieval traces and compute NDCG@10.")]
struct Args {
	#[arg(long)]
	trace: String,
	#[arg(long)]
	sweep: Option<String>,
	#[arg(long, default_value = "")]
	values: String,
	#[arg(long)]
	csv: Option<String>,
	/// Restrict the run to queries whose declared `mode` equals this (e.g. hybrid).
	#[arg(long)]
	mode: Option<String>,
	/// Measure graph-retrieval latency (p50/p95/p99 over the trace, LLM-free)
	/// instead of scoring recall/NDCG. Warmup + timed iterations per query.
	#[arg(long)]
	latency: bool,
	/// Measure retrieval throughput (queries/sec) under N concurrent reader
	/// threads (default: available parallelism). LLM-free.
	#[arg(long)]
	throughput: bool,
	/// Reader threads for --throughput (default: detected parallelism).
	#[arg(long)]
	threads: Option<usize>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();
	let mut t = trace::load(&args.trace)?;

	// `--mode` keeps only queries declaring that mode (docs are untouched, so the
	// graph is identical — only the scored query set narrows).
	if let Some(mode) = &args.mode {
		let want = mode.to_lowercase();
		let total = t.queries.len();
		t.queries.retain(|q| q.mode.to_lowercase() == want);
		if t.queries.is_empty() {
			return Err(format!("--mode {mode}: no queries with that mode (of {total} in the trace)").into());
		}
	}

	let g = build_graph(&t);

	if args.latency {
		let r = kern::bench_support::latency::measure_latency(&g, &RetrievalConfig::default(), &t, 3, 50);
		println!("trace: {}   samples: {}", r.trace_name, r.samples);
		println!(
			"retrieval latency (ms):  mean={:.3}  p50={:.3}  p95={:.3}  p99={:.3}",
			r.mean_ms, r.p50_ms, r.p95_ms, r.p99_ms
		);
		return Ok(());
	}

	if args.throughput {
		let threads = args.threads.unwrap_or_else(|| {
			std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
		});
		let r = kern::bench_support::latency::measure_throughput(
			&g,
			&RetrievalConfig::default(),
			&t,
			threads,
			100,
		);
		println!("trace: {}   threads: {}   queries: {}", r.trace_name, r.threads, r.total_queries);
		println!("retrieval throughput: {:.0} qps  ({:.3}s elapsed)", r.qps, r.elapsed_secs);
		return Ok(());
	}

	match args.sweep {
		None => {
			let report = replay(&g, &RetrievalConfig::default(), &t);
			println!("trace: {}", report.trace_name);
			println!("queries: {}", report.per_query.len());
			for q in &report.per_query {
				println!(
					"  {:<20} mode={:?} ndcg@10={:.4} recall@10={:.4}",
					q.id, q.mode, q.ndcg10, q.recall10
				);
			}
			println!("mean NDCG@10:   {:.4}", report.mean_ndcg10);
			println!("mean recall@10: {:.4}", report.mean_recall10);
		}
		Some(name) => {
			// Validate up front so the user gets a clear message instead of an empty
			// parse result or a bare "required" after the fact.
			if args.values.trim().is_empty() {
				return Err(
					"--values is required for a sweep (comma-separated numbers, e.g. --values 10,20,40)".into(),
				);
			}
			let param = SweepParam::parse(&name)
				.ok_or_else(|| format!("unknown sweep param: {name}"))?;
			let values: Vec<f64> = args
				.values
				.split(',')
				.filter(|s| !s.is_empty())
				.map(|s| s.trim().parse::<f64>())
				.collect::<Result<_, _>>()?;
			if values.is_empty() {
				return Err("--values required for sweep".into());
			}
			let rows = sweep(&g, &t, param, &values);
			let csv = to_csv(&rows);
			match args.csv.as_deref() {
				Some(path) => {
					std::fs::write(path, &csv)?;
					println!("wrote {} sweep rows to {}", rows.len(), path);
				}
				None => print!("{csv}"),
			}
		}
	}
	Ok(())
}
