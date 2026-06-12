use criterion::{black_box, criterion_group, criterion_main, Criterion};
use kern::bench_support::build::build_graph;
use kern::bench_support::replay::replay;
use kern::bench_support::sweep::{sweep, SweepParam};
use kern::bench_support::trace;
use kern::config::RetrievalConfig;

const TRACE_PATH: &str = "benches/retrieval_traces/synthetic.json";

fn bench_replay(c: &mut Criterion) {
	let t = trace::load(TRACE_PATH).expect("synthetic trace must exist");
	let g = build_graph(&t);
	let cfg = RetrievalConfig::default();

	let initial = replay(&g, &cfg, &t);
	eprintln!(
		"retrieval_trace_bench: {} queries, mean NDCG@10 = {:.4}",
		initial.per_query.len(),
		initial.mean_ndcg10
	);

	c.bench_function("retrieval_trace_replay", |b| {
		b.iter(|| replay(black_box(&g), black_box(&cfg), black_box(&t)));
	});
}

fn bench_sweep_rrf_k(c: &mut Criterion) {
	let t = trace::load(TRACE_PATH).expect("synthetic trace must exist");
	let g = build_graph(&t);
	let values = [10.0, 30.0, 60.0, 120.0];

	let rows = sweep(&g, &t, SweepParam::RrfK, &values);
	for r in &rows {
		eprintln!(
			"sweep rrf_k={} -> mean NDCG@10 = {:.4}",
			r.value, r.mean_ndcg10
		);
	}

	c.bench_function("retrieval_trace_sweep_rrf_k", |b| {
		b.iter(|| sweep(black_box(&g), black_box(&t), SweepParam::RrfK, &values));
	});
}

criterion_group!(benches, bench_replay, bench_sweep_rrf_k);
criterion_main!(benches);
