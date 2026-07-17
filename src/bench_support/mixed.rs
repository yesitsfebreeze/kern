//! Mixed read/write/persist contention bench over the real locked store paths —
//! the worst read stall is the number that moves when a writer/flush pins the lock.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::base::accept::accept;
use crate::base::graph::GraphGnn;
use crate::base::types::{ChunkPart, ChunkPartKind, Entity, EntityKind};
use crate::config::{Config, RetrievalConfig};
use crate::retrieval::seed::Mode;

use super::build::build_graph;
use super::embed;
use super::trace::Trace;

type GraphLock = parking_lot::RwLock<GraphGnn>;

#[derive(Debug, Clone)]
pub struct MixedReport {
	pub trace_name: String,
	pub readers: usize,
	pub writers: usize,
	pub duration_secs: f64,
	pub reads: u64,
	pub writes: u64,
	pub persists: u64,
	pub read_qps: f64,
	pub write_ops: f64,
	pub read_p50_ms: f64,
	pub read_p95_ms: f64,
	pub read_p99_ms: f64,
	pub read_max_ms: f64,
}

/// `readers` query threads + `writers` accept threads + one persist thread. The
/// graph is bound to a throwaway on-disk store so persist runs the real LMDB flush.
pub fn measure_mixed(
	trace: &Trace,
	cfg: &RetrievalConfig,
	readers: usize,
	writers: usize,
	duration_secs: f64,
) -> MixedReport {
	let readers = readers.max(1);

	let mut g = build_graph(trace);

	// tempfile is a dev-dependency (unavailable in this bin): mint the dir by hand.
	let dir = std::env::temp_dir().join(format!(
		"kern-mixed-bench-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map(|d| d.as_nanos())
			.unwrap_or(0)
	));
	let _ = std::fs::create_dir_all(&dir);
	let data_dir = dir.to_string_lossy().into_owned();
	if let Ok(store) = crate::base::store::Store::open(&data_dir) {
		g.set_store(Arc::new(store));
	}

	let graph: Arc<GraphLock> = Arc::new(GraphLock::new(g));

	// Query set + writer corpus, precomputed once so no thread re-embeds in the loop.
	let queries: Vec<(String, Vec<f32>, Mode)> = trace
		.queries
		.iter()
		.map(|q| {
			(
				q.query.clone(),
				embed::embed(&q.query),
				Mode::parse(&q.mode),
			)
		})
		.collect();
	let corpus: Vec<Vec<f32>> = trace.docs.iter().map(|d| embed::embed(&d.text)).collect();
	let root_id = crate::base::locks::read_recovered(&graph).root.id.clone();

	let stop = Arc::new(AtomicBool::new(false));
	let writes = Arc::new(AtomicU64::new(0));
	let persists = Arc::new(AtomicU64::new(0));

	let start = Instant::now();
	let latencies: Vec<f64> = std::thread::scope(|s| {
		for w in 0..writers {
			let graph = Arc::clone(&graph);
			let corpus = &corpus;
			let root_id = root_id.clone();
			let stop = Arc::clone(&stop);
			let writes = Arc::clone(&writes);
			s.spawn(move || {
				let mut n = 0u64;
				while !stop.load(Ordering::Relaxed) {
					let vec = corpus[(n as usize) % corpus.len()].clone();
					let id = format!("mix-w{w}-{n}");
					let ent = synthetic_entity(&id, vec);
					{
						let mut gg = crate::base::locks::write_recovered(&graph);
						accept(&mut gg, &root_id, ent, "");
					}
					writes.fetch_add(1, Ordering::Relaxed);
					n += 1;
				}
			});
		}

		{
			let graph = Arc::clone(&graph);
			let stop = Arc::clone(&stop);
			let persists = Arc::clone(&persists);
			let pcfg = Config {
				data_dir: data_dir.clone(),
				..Config::default()
			};
			s.spawn(move || {
				while !stop.load(Ordering::Relaxed) {
					sleep_interruptible(&stop, Duration::from_secs(2));
					if stop.load(Ordering::Relaxed) {
						break;
					}
					crate::commands::save_graph_guarded(&graph, &pcfg);
					persists.fetch_add(1, Ordering::Relaxed);
				}
			});
		}

		let mut handles = Vec::with_capacity(readers);
		for _ in 0..readers {
			let graph = Arc::clone(&graph);
			let queries = &queries;
			let stop = Arc::clone(&stop);
			handles.push(s.spawn(move || {
				let mut samples: Vec<f64> = Vec::new();
				let mut qi = 0usize;
				while !stop.load(Ordering::Relaxed) {
					let (text, qvec, mode) = &queries[qi % queries.len()];
					let t0 = Instant::now();
					let _ = crate::retrieval::answer::query_locked(
						&graph, cfg, qvec, text, *mode, None, None, None,
					);
					samples.push(t0.elapsed().as_secs_f64() * 1000.0);
					qi += 1;
				}
				samples
			}));
		}

		sleep_interruptible(&stop, Duration::from_secs_f64(duration_secs.max(0.1)));
		stop.store(true, Ordering::Relaxed);

		let mut all = Vec::new();
		for h in handles {
			all.extend(h.join().unwrap_or_default());
		}
		all
	});
	let elapsed = start.elapsed().as_secs_f64();

	let _ = std::fs::remove_dir_all(&dir);

	let reads = latencies.len() as u64;
	let mut sorted = latencies;
	sorted.sort_by(crate::base::util::cmp_partial);
	use crate::base::util::percentile_sorted;
	let read_max_ms = sorted.last().copied().unwrap_or(0.0);

	MixedReport {
		trace_name: trace.name.clone(),
		readers,
		writers,
		duration_secs: elapsed,
		reads,
		writes: writes.load(Ordering::Relaxed),
		persists: persists.load(Ordering::Relaxed),
		read_qps: if elapsed > 0.0 {
			reads as f64 / elapsed
		} else {
			0.0
		},
		write_ops: if elapsed > 0.0 {
			writes.load(Ordering::Relaxed) as f64 / elapsed
		} else {
			0.0
		},
		read_p50_ms: percentile_sorted(&sorted, 0.50).unwrap_or(0.0),
		read_p95_ms: percentile_sorted(&sorted, 0.95).unwrap_or(0.0),
		read_p99_ms: percentile_sorted(&sorted, 0.99).unwrap_or(0.0),
		read_max_ms,
	}
}

/// Sleep up to `dur`, but wake early (in short slices) once `stop` is set so a
/// finished run doesn't wait out the whole 2s persist interval.
fn sleep_interruptible(stop: &AtomicBool, dur: Duration) {
	let deadline = Instant::now() + dur;
	while Instant::now() < deadline {
		if stop.load(Ordering::Relaxed) {
			return;
		}
		std::thread::sleep(Duration::from_millis(20));
	}
}

/// A synthetic Claim entity in the same shape as [`build`](super::build)'s doc
/// inserts, so accept treats it like a real ingested chunk.
fn synthetic_entity(id: &str, vec: Vec<f32>) -> Entity {
	Entity {
		id: id.to_string(),
		statements: vec![id.to_string()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector: vec,
		score: 0.5,
		kind: EntityKind::Claim,
		..Default::default()
	}
}
