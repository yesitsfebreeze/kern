use criterion::{black_box, criterion_group, criterion_main, Criterion};

use kern::base::graph::GraphGnn;
use kern::base::hnsw::{AdaptiveEfConfig, HnswIndex};
use kern::base::math::cosine;
use kern::base::search::search_all_unlocked;
use kern::base::types::*;
use kern::config::RetrievalConfig;

fn stub_embed(text: &str) -> Vec<f64> {
	let h = kern::base::util::content_hash(text);
	let bytes = h.as_bytes();
	let mut vec = [0.0f64; 4];
	for i in 0..4 {
		let hi = hex_val(bytes[i * 2]);
		let lo = hex_val(bytes[i * 2 + 1]);
		vec[i] = (hi * 16 + lo) as f64 / 255.0 - 0.5;
	}
	let norm: f64 = vec.iter().map(|v| v * v).sum::<f64>().sqrt();
	if norm > 0.0 {
		for v in &mut vec {
			*v /= norm;
		}
	}
	vec.to_vec()
}

fn hex_val(c: u8) -> u8 {
	match c {
		b'0'..=b'9' => c - b'0',
		b'a'..=b'f' => c - b'a' + 10,
		_ => 0,
	}
}

fn make_entity(id: &str, text: &str) -> Entity {
	Entity {
		id: id.to_string(),
		statements: vec![text.to_string()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector: stub_embed(text),
		score: 0.5,
		kind: EntityKind::Claim,
		..Default::default()
	}
}

fn build_graph(n: usize) -> GraphGnn {
	let mut g = GraphGnn::new();
	for i in 0..n {
		let t = make_entity(
			&format!("t{i}"),
			&format!("thought number {i} about topic {}", i % 10),
		);
		g.root.entities.insert(t.id.clone(), t);
	}
	for i in 0..(n - 1) {
		let from = format!("t{i}");
		let to = format!("t{}", i + 1);
		let from_vec = g.root.entities.get(&from).unwrap().vector.clone();
		let to_vec = g.root.entities.get(&to).unwrap().vector.clone();
		let score = cosine(&from_vec, &to_vec);
		let rid = kern::base::math::reason_id(&from, &to, ReasonKind::Similarity, "", "");
		let reason = Reason {
			id: rid,
			from: from.clone(),
			to: to.clone(),
			kind: ReasonKind::Similarity,
			vector: kern::base::math::average_vec(&from_vec, &to_vec),
			score,
			..Default::default()
		};
		kern::base::reason::add_reason(&mut g.root, reason);
	}
	g.rebuild_index();
	g
}

fn bench_cosine_768(c: &mut Criterion) {
	let a: Vec<f64> = (0..768).map(|i| (i as f64 * 0.001).sin()).collect();
	let b: Vec<f64> = (0..768).map(|i| (i as f64 * 0.002).cos()).collect();
	c.bench_function("cosine_768", |bench| {
		bench.iter(|| cosine(black_box(&a), black_box(&b)));
	});
}

fn bench_search_100(c: &mut Criterion) {
	let g = build_graph(100);
	let query = stub_embed("topic 5");
	c.bench_function("search_100", |bench| {
		bench.iter(|| search_all_unlocked(black_box(&g), black_box(&query), 5));
	});
}

fn bench_search_500(c: &mut Criterion) {
	let g = build_graph(500);
	let query = stub_embed("topic 5");
	c.bench_function("search_500", |bench| {
		bench.iter(|| search_all_unlocked(black_box(&g), black_box(&query), 5));
	});
}

fn bench_query_full_100(c: &mut Criterion) {
	let g = build_graph(100);
	let query = stub_embed("topic 5");
	c.bench_function("query_full_100", |bench| {
		bench.iter(|| {
			kern::retrieval::answer::query(
				black_box(&g),
				&RetrievalConfig::default(),
				black_box(&query),
				"topic 5",
				kern::retrieval::seed::Mode::Hybrid,
				None,
				None,
				None,
			)
		});
	});
}

fn bench_query_full_500(c: &mut Criterion) {
	let g = build_graph(500);
	let query = stub_embed("topic 5");
	c.bench_function("query_full_500", |bench| {
		bench.iter(|| {
			kern::retrieval::answer::query(
				black_box(&g),
				&RetrievalConfig::default(),
				black_box(&query),
				"topic 5",
				kern::retrieval::seed::Mode::Hybrid,
				None,
				None,
				None,
			)
		});
	});
}

fn bench_gnn_tensor_matmul(c: &mut Criterion) {
	let a = kern::gnn::tensor::Tensor::rand(64, 128, 1.0);
	let b = kern::gnn::tensor::Tensor::rand(128, 64, 1.0);
	c.bench_function("tensor_matmul_64x128_128x64", |bench| {
		bench.iter(|| kern::gnn::tensor::Tensor::matmul(black_box(&a), black_box(&b)));
	});
}

fn bench_persist_save_100(c: &mut Criterion) {
	let dir = std::env::temp_dir().join("kern_bench_persist");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();

	let mut g = build_graph(100);
	g.data_dir = dir.to_str().unwrap().to_string();

	c.bench_function("persist_save_100", |bench| {
		bench.iter(|| {
			kern::base::persist::save_all(black_box(&g)).unwrap();
		});
	});

	let _ = std::fs::remove_dir_all(&dir);
}

fn synthetic_lists(num_peers: usize, per_peer: usize) -> Vec<Vec<kern::base::search::EntityHit>> {
	use kern::base::search::EntityHit;
	(0..num_peers)
		.map(|p| {
			(0..per_peer)
				.map(|i| EntityHit {
					entity_id: format!("t{}", (p + i) % per_peer),
					score: 0.1 + ((p * 7 + i * 3) % 100) as f64 / 100.0,
				})
				.collect()
		})
		.collect()
}

fn max_merge_hits(
	lists: &[&[kern::base::search::EntityHit]],
) -> Vec<kern::base::search::EntityHit> {
	use std::collections::HashMap;
	let mut map: HashMap<String, f64> = HashMap::new();
	for list in lists {
		for h in list.iter() {
			let e = map.entry(h.entity_id.clone()).or_insert(f64::NEG_INFINITY);
			if h.score > *e {
				*e = h.score;
			}
		}
	}
	map
		.into_iter()
		.map(|(id, score)| kern::base::search::EntityHit { entity_id: id, score })
		.collect()
}

fn bench_merge_max_vs_softmax(c: &mut Criterion) {
	let data = synthetic_lists(8, 128);
	let refs: Vec<&[_]> = data.iter().map(|v| v.as_slice()).collect();

	c.bench_function("merge_max_8x128", |b| {
		b.iter(|| {
			let _ = black_box(max_merge_hits(black_box(&refs)));
		});
	});

	c.bench_function("merge_softmax_online_8x128", |b| {
		b.iter(|| {
			let _ = black_box(kern::gossip::merge::online_softmax_merge_hits(
				black_box(&refs),
				usize::MAX,
			));
		});
	});

	let max_ranked = {
		let mut v = max_merge_hits(&refs);
		v.sort_by(|a, b| {
			b.score
				.partial_cmp(&a.score)
				.unwrap_or(std::cmp::Ordering::Equal)
		});
		v
	};
	let soft_ranked = kern::gossip::merge::online_softmax_merge_hits(&refs, usize::MAX);
	let top_max: std::collections::HashSet<_> = max_ranked
		.iter()
		.take(10)
		.map(|h| h.entity_id.clone())
		.collect();
	let top_soft: std::collections::HashSet<_> = soft_ranked
		.iter()
		.take(10)
		.map(|h| h.entity_id.clone())
		.collect();
	eprintln!(
		"merge_quality: top-10 jaccard = {:.3}, softmax promoted {} new ids",
		top_max.intersection(&top_soft).count() as f64 / 10.0,
		top_soft.difference(&top_max).count()
	);
}

fn bench_hnsw_adaptive_vs_fixed(c: &mut Criterion) {
	use rand::rngs::StdRng;
	use rand::{RngExt, SeedableRng};
	const DIM: usize = 64;
	const N: usize = 5_000;
	const CLUSTERS: usize = 40;
	const QUERIES: usize = 200;
	const K: usize = 10;
	const EF_FIXED: usize = 200;

	let mut rng = StdRng::seed_from_u64(0xA5A5_5A5A);
	let normalize = |mut v: Vec<f64>| -> Vec<f64> {
		let n: f64 = v.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-18);
		for x in &mut v {
			*x /= n;
		}
		v
	};

	let centroids: Vec<Vec<f64>> = (0..CLUSTERS)
		.map(|_| normalize((0..DIM).map(|_| rng.random::<f64>() - 0.5).collect()))
		.collect();

	let mut idx = HnswIndex::new(16, 200);
	for i in 0..N {
		let c = &centroids[i % CLUSTERS];
		let v: Vec<f64> = c
			.iter()
			.map(|x| x + (rng.random::<f64>() - 0.5) * 0.2)
			.collect();
		idx.insert(format!("v{i}"), normalize(v));
	}

	let queries: Vec<Vec<f64>> = (0..QUERIES)
		.map(|i| {
			if i % 10 < 7 {
				let c = &centroids[i % CLUSTERS];
				let v: Vec<f64> = c
					.iter()
					.map(|x| x + (rng.random::<f64>() - 0.5) * 0.1)
					.collect();
				normalize(v)
			} else {
				let a = &centroids[i % CLUSTERS];
				let b = &centroids[(i + 1) % CLUSTERS];
				let v: Vec<f64> = a.iter().zip(b).map(|(x, y)| 0.5 * (x + y)).collect();
				normalize(v)
			}
		})
		.collect();

	let cfg = AdaptiveEfConfig {
		ef_start: 16,
		ef_max: EF_FIXED,
		ef_step: EF_FIXED,
		spread_epsilon: 0.02,
	};

	let mut fixed_ns: Vec<u128> = Vec::with_capacity(QUERIES);
	let mut adaptive_ns: Vec<u128> = Vec::with_capacity(QUERIES);
	let mut overlap_sum = 0usize;
	for q in &queries {
		let t0 = std::time::Instant::now();
		let f = idx.search(q, K, EF_FIXED);
		fixed_ns.push(t0.elapsed().as_nanos());

		let t1 = std::time::Instant::now();
		let a = idx.search_adaptive(q, K, cfg);
		adaptive_ns.push(t1.elapsed().as_nanos());

		let fset: std::collections::HashSet<_> = f.iter().map(|h| h.id.clone()).collect();
		let aset: std::collections::HashSet<_> = a.iter().map(|h| h.id.clone()).collect();
		overlap_sum += fset.intersection(&aset).count();
	}
	fixed_ns.sort_unstable();
	adaptive_ns.sort_unstable();
	let pct = |v: &[u128], p: f64| v[((v.len() as f64 - 1.0) * p) as usize];
	let recall = overlap_sum as f64 / (QUERIES * K) as f64;
	let reduction = 100.0 * (1.0 - pct(&adaptive_ns, 0.5) as f64 / pct(&fixed_ns, 0.5) as f64);
	eprintln!(
		"hnsw_adaptive_ef: N={} K={} QUERIES={} recall@{}={:.4} p50 fixed={}ns adaptive={}ns ({:.1}% reduction) p95 fixed={}ns adaptive={}ns",
		N, K, QUERIES, K, recall,
		pct(&fixed_ns, 0.5), pct(&adaptive_ns, 0.5), reduction,
		pct(&fixed_ns, 0.95), pct(&adaptive_ns, 0.95),
	);

	c.bench_function("hnsw_search_fixed_ef128", |b| {
		b.iter(|| {
			for q in &queries {
				black_box(idx.search(black_box(q), K, EF_FIXED));
			}
		});
	});
	c.bench_function("hnsw_search_adaptive_ef", |b| {
		b.iter(|| {
			for q in &queries {
				black_box(idx.search_adaptive(black_box(q), K, cfg));
			}
		});
	});
}

criterion_group!(
	benches,
	bench_cosine_768,
	bench_search_100,
	bench_search_500,
	bench_query_full_100,
	bench_query_full_500,
	bench_gnn_tensor_matmul,
	bench_persist_save_100,
	bench_merge_max_vs_softmax,
	bench_hnsw_adaptive_vs_fixed,
);
criterion_main!(benches);
