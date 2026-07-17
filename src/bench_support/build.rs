//! Benchmark graph construction: turn a replay `Trace`'s documents into a
//! `GraphGnn` with similarity edges. The replay/scoring loop lives in `replay.rs`.

use crate::base::graph::GraphGnn;
use crate::base::math::{average_vec, cosine, reason_id};
use crate::base::reason::add_reason;
use crate::base::types::*;

use super::embed;
use super::trace::Trace;

pub fn build_graph(trace: &Trace) -> GraphGnn {
	let mut g = GraphGnn::new();
	let root_id = g.root.id.clone();
	insert_docs(&mut g, &root_id, trace);
	seed_similarity_edges(&mut g, &root_id, trace);
	g.rebuild_index();
	// Populate the BM25 index as the real load path does; empty, "hybrid" queries
	// silently fall back to dense-only (see build_graph_populates_a_searchable_lexical_index).
	if let Some(lex) = g.lexical() {
		lex.rebuild_from_graph(&g);
	}
	g
}

fn insert_docs(g: &mut GraphGnn, root_id: &str, trace: &Trace) {
	for doc in &trace.docs {
		let vec = embed::embed(&doc.text);
		let kind = doc
			.kind
			.as_deref()
			.and_then(EntityKind::parse)
			.unwrap_or(EntityKind::Claim);
		let t = Entity {
			id: doc.id.clone(),
			statements: vec![doc.text.clone()],
			chunks: vec![ChunkPart {
				kind: ChunkPartKind::StatementRef,
				text: String::new(),
				index: 0,
			}],
			vector: vec,
			score: 0.5,
			kind,
			..Default::default()
		};
		if let Some(kern) = g.kerns.get_mut(root_id) {
			kern.entities.insert(t.id.clone(), t);
		}
	}
}

/// Cosine floor for a similarity edge. A looser floor densifies the graph and
/// tanks ranking (NDCG) without changing recall.
const SIMILARITY_EDGE_FLOOR: f64 = 0.5;

/// Seed similarity edges for every pair clearing [`SIMILARITY_EDGE_FLOOR`].
/// O(n^2) on purpose (see pairwise_seeding_matches_ann_top_k_1k).
fn seed_similarity_edges(g: &mut GraphGnn, root_id: &str, trace: &Trace) {
	use rayon::prelude::*;

	let kern = g.kerns.get(root_id).expect("root kern exists");
	let docs: Vec<(&str, &[f32])> = trace
		.docs
		.iter()
		.map(|d| {
			(
				d.id.as_str(),
				kern
					.entities
					.get(&d.id)
					.expect("inserted above")
					.vector
					.as_slice(),
			)
		})
		.collect();

	let reasons: Vec<Reason> = (0..docs.len())
		.into_par_iter()
		.flat_map_iter(|i| {
			let (from, from_vec) = docs[i];
			docs[i + 1..].iter().filter_map(move |&(to, to_vec)| {
				let score = cosine(from_vec, to_vec);
				if score < SIMILARITY_EDGE_FLOOR {
					return None;
				}
				Some(Reason {
					id: reason_id(from, to, ReasonKind::Similarity, "", ""),
					from: from.to_string(),
					to: to.to_string(),
					kind: ReasonKind::Similarity,
					vector: average_vec(from_vec, to_vec),
					score,
					..Default::default()
				})
			})
		})
		.collect();

	if let Some(kern) = g.kerns.get_mut(root_id) {
		for r in reasons {
			add_reason(kern, r);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::util::content_hash;
	use crate::bench_support::trace::{Trace, TraceDoc};
	use std::collections::HashSet;

	/// 1000 docs: 40 clusters of 3 near-identical docs (siblings cosine ~0.89 >
	/// FLOOR) among 880 unrelated singles — the only >FLOOR pairs are 120 edges.
	fn thousand_doc_clustered_trace() -> Trace {
		let mut docs = Vec::new();
		for c in 0..40 {
			let shared: String = (0..8).map(|t| format!("clus{c}tok{t} ")).collect();
			for m in 0..3 {
				docs.push(TraceDoc {
					id: format!("clus{c}_m{m}"),
					text: format!("{shared}clus{c}uniq{m}"),
					kind: None,
				});
			}
		}
		for s in 0..880 {
			let text: String = (0..6).map(|t| format!("solo{s}word{t} ")).collect();
			docs.push(TraceDoc {
				id: format!("solo{s}"),
				text: text.trim_end().to_string(),
				kind: None,
			});
		}
		Trace {
			name: "edge-seeding-equivalence-1k".into(),
			docs,
			queries: vec![],
		}
	}

	fn pairwise_edge_ids(g: &GraphGnn) -> HashSet<String> {
		let root = g.root.id.clone();
		g.kerns
			.get(&root)
			.expect("root kern")
			.reasons
			.keys()
			.cloned()
			.collect()
	}

	/// The edge set an ANN top-k seeder WOULD produce: probe each doc's index
	/// neighbourhood, then apply the same exact cosine + floor as the pairwise scan.
	fn ann_top_k_edge_ids(g: &GraphGnn, trace: &Trace) -> HashSet<String> {
		const NEIGHBOR_K: usize = 64;
		const NEIGHBOR_EF: usize = 256;
		let root = g.root.id.clone();
		let kern = g.kerns.get(&root).expect("root kern");
		let vecs: Vec<(&str, &[f32])> = trace
			.docs
			.iter()
			.map(|d| {
				(
					d.id.as_str(),
					kern
						.entities
						.get(&d.id)
						.expect("inserted")
						.vector
						.as_slice(),
				)
			})
			.collect();
		let pos: std::collections::HashMap<&str, usize> = vecs
			.iter()
			.enumerate()
			.map(|(i, (id, _))| (*id, i))
			.collect();
		let mut ids = HashSet::new();
		for (i, (_, vec)) in vecs.iter().enumerate() {
			for h in g.entity_idx.search(vec, NEIGHBOR_K, NEIGHBOR_EF) {
				let Some(&j) = pos.get(h.id.as_str()) else {
					continue;
				};
				if j == i {
					continue;
				}
				let (a, b) = if i < j { (i, j) } else { (j, i) };
				let (from, from_vec) = vecs[a];
				let (to, to_vec) = vecs[b];
				if cosine(from_vec, to_vec) >= SIMILARITY_EDGE_FLOOR {
					ids.insert(reason_id(from, to, ReasonKind::Similarity, "", ""));
				}
			}
		}
		ids
	}

	/// Decision record: ANN top-k seeding was rejected — identical edge set, net
	/// slower. Do not "optimize" the O(n^2) scan away without re-running this.
	#[test]
	fn pairwise_seeding_matches_ann_top_k_1k() {
		let trace = thousand_doc_clustered_trace();
		let g = build_graph(&trace);

		let pairwise = pairwise_edge_ids(&g);
		let ann = ann_top_k_edge_ids(&g, &trace);

		assert_eq!(
			pairwise.len(),
			120,
			"40 clusters x (3 choose 2) = 120 edges"
		);
		assert_eq!(
			pairwise,
			ann,
			"ANN top-k must recover exactly the pairwise edge set at 1k \
			 (pairwise {} vs ann {}, missing {:?}, extra {:?})",
			pairwise.len(),
			ann.len(),
			pairwise.difference(&ann).collect::<Vec<_>>(),
			ann.difference(&pairwise).collect::<Vec<_>>(),
		);

		let mut sorted: Vec<&String> = pairwise.iter().collect();
		sorted.sort();
		let fingerprint = content_hash(
			&sorted
				.iter()
				.map(|s| s.as_str())
				.collect::<Vec<_>>()
				.join("\n"),
		);
		println!(
			"EDGE-ARTIFACT docs=1000 edges={} fingerprint={fingerprint}",
			pairwise.len()
		);
	}

	#[test]
	fn build_graph_populates_a_searchable_lexical_index() {
		// Regression for the empty-lexical-index bug.
		let trace = Trace {
			name: "t".into(),
			docs: vec![
				TraceDoc {
					id: "go_concurrency".into(),
					text: "Go goroutines channels select concurrency primitives".into(),
					kind: None,
				},
				TraceDoc {
					id: "rust_ownership".into(),
					text: "Rust ownership borrow checker memory safety".into(),
					kind: None,
				},
			],
			queries: vec![],
		};
		let g = build_graph(&trace);
		let lex = g.lexical().expect("graph carries a lexical index");
		let hits = lex.search("go goroutines channels", 5);
		assert!(
			!hits.is_empty(),
			"the lexical index is populated, not empty"
		);
		assert_eq!(
			hits[0].entity_id, "go_concurrency",
			"strong token overlap must rank first, got {hits:?}"
		);
	}
}
