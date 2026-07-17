use clap::Parser;
use kern::bench_support::build::build_graph;
use kern::bench_support::replay::replay;
use kern::bench_support::sweep::{sweep, to_csv, SweepParam};
use kern::bench_support::trace;
use kern::config::RetrievalConfig;

#[derive(Parser, Debug)]
#[command(
	name = "retrieval_bench",
	about = "Replay retrieval traces and compute NDCG@10."
)]
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
	/// Mixed read/write/persist contention run: N reader threads on the locked
	/// query path + M writer threads doing accept() + one persist thread. Reports
	/// read p50/p95/p99, read qps, write ops/s, and the worst single read stall.
	#[arg(long)]
	mixed: bool,
	/// Writer threads for --mixed (default: 2).
	#[arg(long)]
	writers: Option<usize>,
	/// Wall-clock seconds for the --mixed run (default: 10).
	#[arg(long)]
	secs: Option<f64>,
	/// Report the graph's vector-storage footprint (f32 vs int8) instead of
	/// scoring recall/NDCG.
	#[arg(long)]
	memory: bool,
	/// Break the graph-retrieval path into per-stage timings (seed/fuse/expand/
	/// merge/boosts/mmr/chains) with each stage's mean/p50/p95 and share. LLM-free.
	#[arg(long)]
	profile: bool,
	/// One combined Tier-0 snapshot: corpus size, recall@10/NDCG@10, latency
	/// p50/p95/p99, throughput, and vector memory — in a single run.
	#[arg(long)]
	all: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();
	let mut t = trace::load(&args.trace)?;

	if let Some(mode) = &args.mode {
		let want = mode.to_lowercase();
		let total = t.queries.len();
		t.queries.retain(|q| q.mode.to_lowercase() == want);
		if t.queries.is_empty() {
			return Err(
				format!("--mode {mode}: no queries with that mode (of {total} in the trace)").into(),
			);
		}
	}

	let g = build_graph(&t);

	if args.latency {
		let r =
			kern::bench_support::latency::measure_latency(&g, &RetrievalConfig::default(), &t, 3, 50);
		println!("trace: {}   samples: {}", r.trace_name, r.samples);
		println!(
			"retrieval latency (ms):  mean={:.3}  p50={:.3}  p95={:.3}  p99={:.3}",
			r.mean_ms, r.p50_ms, r.p95_ms, r.p99_ms
		);
		return Ok(());
	}

	if args.all {
		use kern::bench_support::{latency, memory};
		let mem = memory::estimate_memory(&g);
		let rep = replay(&g, &RetrievalConfig::default(), &t);
		let lat = latency::measure_latency(&g, &RetrievalConfig::default(), &t, 3, 50);
		let threads = args.threads.unwrap_or_else(|| {
			std::thread::available_parallelism()
				.map(|n| n.get())
				.unwrap_or(4)
		});
		let tput = latency::measure_throughput(&g, &RetrievalConfig::default(), &t, threads, 100);
		println!("=== Tier-0 snapshot: {} ===", t.name);
		println!(
			"corpus:      {} entities, {} vectors x dim {}",
			mem.entities, mem.vectors, mem.dim
		);
		println!(
			"quality:     recall@10={:.4}  NDCG@10={:.4}   ({} queries)",
			rep.mean_recall10,
			rep.mean_ndcg10,
			rep.per_query.len()
		);
		println!(
			"latency ms:  mean={:.3}  p50={:.3}  p95={:.3}  p99={:.3}",
			lat.mean_ms, lat.p50_ms, lat.p95_ms, lat.p99_ms
		);
		println!(
			"throughput:  {:.0} qps  ({} threads)",
			tput.qps, tput.threads
		);
		println!(
			"memory:      vectors f32={:.1} KiB  int8={:.1} KiB  ({:.1}x)",
			mem.float_vector_bytes as f64 / 1024.0,
			mem.int8_vector_bytes as f64 / 1024.0,
			mem.quant_ratio()
		);
		return Ok(());
	}

	if args.profile {
		let r = kern::bench_support::stage_profile::measure_stage_profile(
			&g,
			&RetrievalConfig::default(),
			&t,
			3,
			20,
		);
		print!("{r}");
		return Ok(());
	}

	if args.memory {
		let m = kern::bench_support::memory::estimate_memory(&g);
		println!(
			"trace: {}   entities: {}   vectors: {}   dim: {}",
			t.name, m.entities, m.vectors, m.dim
		);
		println!(
			"vector storage:  f32={:.1} KiB   int8={:.1} KiB   ratio={:.1}x",
			m.float_vector_bytes as f64 / 1024.0,
			m.int8_vector_bytes as f64 / 1024.0,
			m.quant_ratio()
		);
		return Ok(());
	}

	if args.mixed {
		let readers = args.threads.unwrap_or_else(|| {
			std::thread::available_parallelism()
				.map(|n| n.get())
				.unwrap_or(4)
		});
		let writers = args.writers.unwrap_or(2);
		let secs = args.secs.unwrap_or(10.0);
		let r = kern::bench_support::mixed::measure_mixed(
			&t,
			&RetrievalConfig::default(),
			readers,
			writers,
			secs,
		);
		println!(
			"trace: {}   readers: {}   writers: {}   ({:.1}s)",
			r.trace_name, r.readers, r.writers, r.duration_secs
		);
		println!(
			"reads: {}  ({:.0} qps)   writes: {}  ({:.0} ops/s)   persists: {}",
			r.reads, r.read_qps, r.writes, r.write_ops, r.persists
		);
		println!(
			"read latency (ms):  p50={:.3}  p95={:.3}  p99={:.3}  max={:.3}",
			r.read_p50_ms, r.read_p95_ms, r.read_p99_ms, r.read_max_ms
		);
		return Ok(());
	}

	if args.throughput {
		let threads = args.threads.unwrap_or_else(|| {
			std::thread::available_parallelism()
				.map(|n| n.get())
				.unwrap_or(4)
		});
		let r = kern::bench_support::latency::measure_throughput(
			&g,
			&RetrievalConfig::default(),
			&t,
			threads,
			100,
		);
		println!(
			"trace: {}   threads: {}   queries: {}",
			r.trace_name, r.threads, r.total_queries
		);
		println!(
			"retrieval throughput: {:.0} qps  ({:.3}s elapsed)",
			r.qps, r.elapsed_secs
		);
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
			let param = SweepParam::parse(&name).ok_or_else(|| format!("unknown sweep param: {name}"))?;
			let values: Vec<f64> = args
				.values
				.split(',')
				.map(str::trim)
				.filter(|s| !s.is_empty())
				.map(|s| s.parse::<f64>())
				.collect::<Result<_, _>>()?;
			if values.is_empty() {
				return Err(
					"--values is required for a sweep (comma-separated numbers, e.g. --values 10,20,40)"
						.into(),
				);
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
