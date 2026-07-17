use std::time::Instant;

use crate::base::types::EntityKind;

use super::backend::{Doc, VectorBackend};
use super::embed;
use super::ndcg;

#[derive(Debug, Clone)]
pub struct CompareQuery {
	pub id: String,
	pub vector: Vec<f32>,
	pub expected_ids: Vec<String>,
	pub kind_filter: Option<EntityKind>,
}

pub struct Corpus {
	pub docs: Vec<Doc>,
	pub queries: Vec<CompareQuery>,
}

impl Corpus {
	pub fn synthetic(n_docs: usize, n_queries: usize, seed: u64) -> Self {
		let vocab: Vec<String> = (0..200).map(|i| format!("term{i}")).collect();
		let mut s = seed | 1; // xorshift needs a non-zero state
		let mut next = move || {
			s ^= s << 13;
			s ^= s >> 7;
			s ^= s << 17;
			s
		};

		let mut doc_tokens: Vec<Vec<usize>> = Vec::with_capacity(n_docs);
		let docs: Vec<Doc> = (0..n_docs)
			.map(|i| {
				let toks: Vec<usize> = (0..8).map(|_| (next() as usize) % vocab.len()).collect();
				let text = toks
					.iter()
					.map(|&t| vocab[t].as_str())
					.collect::<Vec<_>>()
					.join(" ");
				doc_tokens.push(toks);
				Doc {
					id: format!("doc{i}"),
					vector: embed::embed(&text),
					kind: Some(EntityKind::Claim),
				}
			})
			.collect();

		let queries: Vec<CompareQuery> = (0..n_queries)
			.map(|q| {
				let target = (next() as usize) % n_docs.max(1);
				let text = doc_tokens[target]
					.iter()
					.take(4)
					.map(|&t| vocab[t].as_str())
					.collect::<Vec<_>>()
					.join(" ");
				CompareQuery {
					id: format!("q{q}"),
					vector: embed::embed(&text),
					expected_ids: vec![format!("doc{target}")],
					kind_filter: None,
				}
			})
			.collect();

		Corpus { docs, queries }
	}
}

#[derive(Debug, Clone)]
pub struct BackendReport {
	pub name: String,
	pub mean_recall10: f64,
	pub mean_ndcg10: f64,
	pub mean_latency_ms: f64,
	pub vector_bytes: usize,
}

pub const K: usize = 10;

pub fn compare(backends: &mut [Box<dyn VectorBackend>], corpus: &Corpus) -> Vec<BackendReport> {
	let mut out = Vec::with_capacity(backends.len());
	for b in backends.iter_mut() {
		b.index(&corpus.docs);
		let mut recall_sum = 0.0;
		let mut ndcg_sum = 0.0;
		let mut lat_sum = 0.0;
		for q in &corpus.queries {
			let t0 = Instant::now();
			let hits = b.query(&q.vector, K, q.kind_filter);
			lat_sum += t0.elapsed().as_secs_f64() * 1000.0;
			let ranked: Vec<String> = hits.into_iter().map(|h| h.id).collect();
			recall_sum += ndcg::recall_at_k(&ranked, &q.expected_ids, K);
			ndcg_sum += ndcg::ndcg_at_k(&ranked, &q.expected_ids, K);
		}
		let n = corpus.queries.len().max(1) as f64;
		out.push(BackendReport {
			name: b.name().to_string(),
			mean_recall10: recall_sum / n,
			mean_ndcg10: ndcg_sum / n,
			mean_latency_ms: lat_sum / n,
			vector_bytes: b.vector_bytes(),
		});
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bench_support::backend::{BruteForceBackend, KernBackend};

	fn doc(id: &str, v: Vec<f32>, kind: EntityKind) -> Doc {
		Doc {
			id: id.into(),
			vector: v,
			kind: Some(kind),
		}
	}

	fn corpus() -> Corpus {
		Corpus {
			docs: vec![
				doc("a", vec![1.0, 0.0, 0.0], EntityKind::Fact),
				doc("b", vec![0.0, 1.0, 0.0], EntityKind::Claim),
				doc("c", vec![0.0, 0.0, 1.0], EntityKind::Claim),
				doc("d", vec![1.0, 1.0, 0.0], EntityKind::Claim),
			],
			queries: vec![
				CompareQuery {
					id: "qa".into(),
					vector: vec![1.0, 0.0, 0.0],
					expected_ids: vec!["a".into()],
					kind_filter: None,
				},
				CompareQuery {
					id: "qb".into(),
					vector: vec![0.0, 1.0, 0.0],
					expected_ids: vec!["b".into()],
					kind_filter: None,
				},
			],
		}
	}

	#[test]
	fn compare_scores_a_backend_through_the_shared_metrics() {
		let mut backends: Vec<Box<dyn VectorBackend>> = vec![Box::new(KernBackend::new())];
		let reports = compare(&mut backends, &corpus());
		assert_eq!(reports.len(), 1);
		assert_eq!(reports[0].name, "kern");
		assert_eq!(
			reports[0].mean_recall10, 1.0,
			"each query's expected doc is its nearest"
		);
		assert!(reports[0].mean_ndcg10 > 0.0);
		assert!(reports[0].mean_latency_ms >= 0.0);
		assert!(reports[0].vector_bytes > 0);
	}

	#[test]
	fn synthetic_corpus_drives_a_scale_comparison() {
		let corpus = Corpus::synthetic(500, 50, 42);
		assert_eq!(corpus.docs.len(), 500);
		assert_eq!(corpus.queries.len(), 50);
		let mut backends: Vec<Box<dyn VectorBackend>> = vec![Box::new(KernBackend::new())];
		let r = compare(&mut backends, &corpus);
		assert!(
			r[0].mean_recall10 > 0.3,
			"scale recall@10 should be substantial, got {}",
			r[0].mean_recall10
		);
		assert!(r[0].vector_bytes > 0);
	}

	#[test]
	fn kern_ann_recall_tracks_exact_brute_force() {
		let corpus = Corpus::synthetic(300, 40, 99);
		let mut kern: Vec<Box<dyn VectorBackend>> = vec![Box::new(KernBackend::new())];
		let mut brute: Vec<Box<dyn VectorBackend>> = vec![Box::new(BruteForceBackend::new())];
		let kr = compare(&mut kern, &corpus)[0].clone();
		let br = compare(&mut brute, &corpus)[0].clone();
		assert!(
			br.mean_recall10 > 0.0,
			"exact search finds the expected docs"
		);
		assert!(
			kr.mean_recall10 >= 0.8 * br.mean_recall10,
			"kern HNSW recall@10 {} should track exact {} (>=80%)",
			kr.mean_recall10,
			br.mean_recall10
		);
	}

	fn mean_target_recall_10(
		queries: &[CompareQuery],
		mut top10: impl FnMut(&CompareQuery) -> Vec<String>,
	) -> f64 {
		if queries.is_empty() {
			return 0.0;
		}
		let hits = queries
			.iter()
			.filter(|q| {
				let top = top10(q);
				q.expected_ids.iter().any(|e| top.contains(e))
			})
			.count();
		hits as f64 / queries.len() as f64
	}

	fn disk_backed_graph(docs: &[Doc], dir: &std::path::Path) -> crate::base::graph::GraphGnn {
		use crate::base::graph::GraphGnn;
		use crate::base::types::{Entity, Kern};
		let mut g = GraphGnn::new();
		g.data_dir = dir.to_string_lossy().into_owned();
		let mut kern = Kern::new("k", "");
		for d in docs {
			kern.entities.insert(
				d.id.clone(),
				Entity {
					id: d.id.clone(),
					vector: d.vector.clone(),
					score: 0.5,
					kind: d.kind.unwrap_or(EntityKind::Claim),
					..Default::default()
				},
			);
		}
		g.kerns.insert("k".to_string(), kern);
		g.set_disk_threshold(0);
		g.rebuild_index();
		g
	}

	#[test]
	fn disk_spilled_path_recall_tracks_the_in_ram_index() {
		let corpus = Corpus::synthetic(300, 40, 99);

		let mut kern = KernBackend::new();
		kern.index(&corpus.docs);
		let ram_recall = mean_target_recall_10(&corpus.queries, |q| {
			kern
				.query(&q.vector, 10, None)
				.into_iter()
				.map(|h| h.id)
				.collect()
		});

		let dir = tempfile::tempdir().unwrap();
		let g = disk_backed_graph(&corpus.docs, dir.path());
		assert!(
			matches!(
				g.entity_idx,
				crate::base::vector_backend::VectorBackend::Disk { .. }
			),
			"corpus spilled to the disk backend"
		);
		let disk_recall = mean_target_recall_10(&corpus.queries, |q| {
			crate::base::search::search_all_unlocked(&g, &q.vector, 10)
				.into_iter()
				.map(|h| h.entity_id)
				.collect()
		});

		assert!(
			disk_recall > 0.3,
			"disk recall@10 should be substantial, got {disk_recall}"
		);
		assert!(
			disk_recall >= 0.8 * ram_recall,
			"disk recall@10 {disk_recall} should track in-RAM {ram_recall} (>=80%)"
		);
	}

	#[test]
	fn synthetic_corpus_is_deterministic_for_a_seed() {
		let a = Corpus::synthetic(20, 5, 7);
		let b = Corpus::synthetic(20, 5, 7);
		assert_eq!(
			a.docs[0].vector, b.docs[0].vector,
			"same seed -> identical doc vectors"
		);
		assert_eq!(
			a.queries[0].expected_ids, b.queries[0].expected_ids,
			"same query targets"
		);
	}

	#[test]
	fn identical_backends_produce_identical_quality_and_memory() {
		let mut backends: Vec<Box<dyn VectorBackend>> =
			vec![Box::new(KernBackend::new()), Box::new(KernBackend::new())];
		let r = compare(&mut backends, &corpus());
		assert_eq!(
			r[0].mean_recall10, r[1].mean_recall10,
			"recall is harness-deterministic"
		);
		assert_eq!(
			r[0].mean_ndcg10, r[1].mean_ndcg10,
			"ndcg is harness-deterministic"
		);
		assert_eq!(
			r[0].vector_bytes, r[1].vector_bytes,
			"memory is harness-deterministic"
		);
	}
}
