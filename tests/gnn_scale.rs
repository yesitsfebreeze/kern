// The scaling instrument behind ROADMAP item 28, in the shape of
// `tests/gc_scale.rs`. Ignored by default: it trains a real GCN over kerns of up
// to a few thousand entities at the production 384-dim / 24-epoch settings,
// which is minutes in release and effectively unbounded in debug.
//
//   cargo test --release --test gnn_scale -- --ignored --nocapture
//
// Two questions, one test each:
//   1. what does one `GnnPropagate` cost as the kern grows (`gnn_train_scale`)
//   2. what waits behind it on the single tick loop (`tick_head_of_line_delay`)
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use kern::base::graph::GraphGnn;
use kern::base::heat::HeatConfig;
use kern::base::reason::add_reason;
use kern::base::types::{Entity, EntityKind, Kern, Reason};
use kern::config::TickConfig;
use kern::gnn::propagate::GnnConfig;
use kern::tick::gnn_propagate::do_gnn_propagate;
use kern::tick::queue::{task, task_commit_access, Queue, TaskKind};

const DIM: usize = 384;

fn dense_vec(seed: usize) -> Vec<f32> {
	let mut h = seed as u64 | 1;
	let mut v: Vec<f32> = (0..DIM)
		.map(|_| {
			h ^= h << 13;
			h ^= h >> 7;
			h ^= h << 17;
			(h % 2_000_000) as f32 / 1_000_000.0 - 1.0
		})
		.collect();
	let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
	for x in &mut v {
		*x /= n;
	}
	v
}

// Ingest gives each accepted entity at most one similarity edge to its top-1
// neighbour (`add_similarity_reason`, `src/base/accept.rs:378-411`), so the
// real graph is sparse: degree ~2. `deg` is the knob because the dense N x N
// adjacency this code builds is insensitive to it — that is the finding.
fn kern_with(n: usize, deg: usize) -> Kern {
	let mut k = Kern::new("kx", "");
	k.graviton_text = "named".into();
	k.graviton_vec = dense_vec(0);
	for i in 0..n {
		let id = format!("e{i:07}");
		k.entities.insert(
			id.clone(),
			Entity {
				id,
				vector: dense_vec(i + 1).into(),
				kind: EntityKind::Claim,
				..Default::default()
			},
		);
	}
	for i in 0..n {
		for d in 1..=deg.max(1) {
			let j = (i + d) % n;
			if i == j {
				continue;
			}
			let (from, to) = (format!("e{i:07}"), format!("e{j:07}"));
			add_reason(
				&mut k,
				Reason {
					id: format!("{from}->{to}"),
					from,
					to,
					..Default::default()
				},
			);
		}
	}
	k
}

fn graph_with(n: usize, deg: usize) -> Arc<RwLock<GraphGnn>> {
	let mut g = GraphGnn::new();
	let k = kern_with(n, deg);
	let ids: Vec<String> = k.entities.keys().cloned().collect();
	g.kerns.insert("kx".into(), k);
	// `commit_access_ids` resolves through `kern_of_entity`, so an unindexed
	// entity makes the CommitAccess probe a silent no-op.
	for id in &ids {
		g.index_entity(id, "kx");
	}
	Arc::new(RwLock::new(g))
}

#[test]
#[ignore = "minutes in release; run explicitly with --ignored"]
fn gnn_train_scale() {
	let cfg = GnnConfig::defaults();
	println!(
		"dim={DIM} min_thoughts={} train_epochs={}",
		cfg.min_thoughts, cfg.train_epochs
	);
	for n in [128usize, 256, 512, 1024, 2048, 4096] {
		let g = graph_with(n, 2);
		let q = Queue::new(512);
		let t = Instant::now();
		do_gnn_propagate(&q, &g, "kx", &cfg);
		let ms = t.elapsed().as_secs_f64() * 1000.0;
		let updated = g
			.read()
			.kerns
			.get("kx")
			.map(|k| {
				k.entities
					.values()
					.filter(|e| !e.gnn_vector.is_empty())
					.count()
			})
			.unwrap_or(0);
		println!(
			"N={n:<6} deg=2  propagate={ms:10.1}ms  ({:6.2}s)  updated={updated}  \
			 adjacency={:.1}MB",
			ms / 1000.0,
			(n * n * 8) as f64 / 1.0e6
		);
	}
}

// Every other maintenance task, for scale: what the tick loop is being asked to
// stall. Sizes are the same N so the numbers are directly comparable.
#[test]
#[ignore = "minutes in release; run explicitly with --ignored"]
fn other_tick_tasks_scale() {
	for n in [1024usize, 4096] {
		let g = graph_with(n, 2);
		let q = Queue::new(512);

		let t = Instant::now();
		kern::tick::stigmergy::run_gc(&g, "kx", &HeatConfig::default());
		let gc_ms = t.elapsed().as_secs_f64() * 1000.0;

		let ids: Vec<String> = g.read().kerns["kx"].entities.keys().cloned().collect();
		let commit = task_commit_access(&ids[..1.min(ids.len())]);
		let t = Instant::now();
		kern::tick::tasks::do_commit_access(&g, &commit.extra, &HeatConfig::default());
		let commit_ms = t.elapsed().as_secs_f64() * 1000.0;

		let t = Instant::now();
		kern::tick::idle::run_idle_sweep(&g, Duration::from_secs(3600));
		let idle_ms = t.elapsed().as_secs_f64() * 1000.0;

		let _ = &q;
		println!(
			"N={n:<6} stigmergy_gc={gc_ms:8.3}ms  commit_access={commit_ms:8.3}ms  \
			 idle_sweep={idle_ms:8.3}ms"
		);
	}
}

// The head-of-line claim itself, on the REAL loop (`kern::tick::start`), not a
// hand-rolled drain: a `GnnPropagate` for a large kern is enqueued first and a
// `CommitAccess` — the recall path's heat write-back, `src/mcp/tools_query.rs:196`
// — immediately behind it. The delay is wall time from enqueue to the access
// landing on the entity.
#[test]
#[ignore = "minutes in release; run explicitly with --ignored"]
fn tick_head_of_line_delay() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	for n in [512usize, 2048] {
		for with_gnn in [false, true] {
			let g = graph_with(n, 2);
			let q = Arc::new(Queue::new(512));
			let ctx = kern::tick::TickContext {
				llm: None,
				embed: None,
				broadcast_q: None,
				gnn_cfg: GnnConfig::defaults(),
				tick_cfg: TickConfig::default(),
				heat_cfg: HeatConfig::default(),
			};
			let _guard = rt.enter();
			let handle = kern::tick::start(q.clone(), g.clone(), ctx);

			let probe = "e0000000".to_string();
			let t = Instant::now();
			if with_gnn {
				assert!(q.enqueue(task(TaskKind::GnnPropagate, "kx")));
			}
			assert!(q.enqueue(task_commit_access(std::slice::from_ref(&probe))));

			let landed = rt.block_on(async {
				for _ in 0..600_000 {
					if g.read().kerns["kx"].entities[&probe].accessed_at.is_some() {
						return true;
					}
					tokio::time::sleep(Duration::from_millis(1)).await;
				}
				false
			});
			assert!(
				landed,
				"CommitAccess never landed; the probe is not measuring"
			);
			let ms = t.elapsed().as_secs_f64() * 1000.0;
			handle.abort();
			let label = if with_gnn { "gnn ahead" } else { "no gnn   " };
			println!("N={n:<6} {label}  commit_access landed after {ms:10.1}ms");
		}
	}
}
