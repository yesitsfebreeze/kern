//! Benchmark graph construction: turn a replay `Trace`'s documents into a
//! `GraphGnn` with similarity edges. Kept separate from the replay/scoring loop
//! (`replay.rs`) so each module owns a single responsibility — build vs measure.

use crate::base::graph::GraphGnn;
use crate::base::math::{average_vec, cosine, reason_id};
use crate::base::reason::add_reason;
use crate::base::types::*;

use super::embed;
use super::trace::Trace;

/// Build a benchmark graph from a trace: insert each document as a Claim entity,
/// seed pairwise similarity edges, then build the ANN index.
pub fn build_graph(trace: &Trace) -> GraphGnn {
	let mut g = GraphGnn::new();
	let root_id = g.root.id.clone();
	insert_docs(&mut g, &root_id, trace);
	seed_similarity_edges(&mut g, &root_id, trace);
	g.rebuild_index();
	// Populate the BM25 lexical index from the docs, mirroring the real load path
	// (graph.rs builds it via rebuild_from_graph). Without this the index exists but
	// is empty, so "hybrid" trace queries silently fall back to dense-only — and the
	// bench's deterministic stub embedder is not semantic, so the lexical leg (and
	// any hybrid recall number) would be a fiction.
	if let Some(lex) = g.lexical() {
		lex.rebuild_from_graph(&g);
	}
	g
}

/// Insert every trace document into the root kern as a Claim entity carrying the
/// deterministic bench embedding of its text.
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

/// Cosine floor for a similarity edge. A REAL graph's reason-edges connect
/// genuinely related entities; a near-orthogonal pair (cosine ~0.1) is not
/// "related". A loose floor wires almost every pair together, and that dense
/// graph lets graph expansion's corroboration boost promote well-connected
/// central nodes over the direct best match — which tanks ranking (NDCG) without
/// changing recall. 0.5 keeps only substantively-similar pairs, matching a
/// realistic edge density (NDCG@10 on synthetic.json: 0.54 at 0.1 -> ~1.0 here).
const SIMILARITY_EDGE_FLOOR: f64 = 0.5;

/// Seed similarity edges between every pair of documents whose cosine clears
/// [`SIMILARITY_EDGE_FLOOR`]. O(n^2) on purpose: benchmark traces are small (tens
/// to low-hundreds of docs), so the full pairwise edge set is cheaper and more
/// faithful than approximating it via the ANN index. If trace corpora ever grow
/// large, replace this with a top-k batch index build.
fn seed_similarity_edges(g: &mut GraphGnn, root_id: &str, trace: &Trace) {
	let ids: Vec<String> = trace.docs.iter().map(|d| d.id.clone()).collect();
	for i in 0..ids.len() {
		for j in (i + 1)..ids.len() {
			let from = ids[i].clone();
			let to = ids[j].clone();
			let kern = g.kerns.get(root_id).expect("root kern exists");
			let from_vec = kern
				.entities
				.get(&from)
				.expect("inserted above")
				.vector
				.clone();
			let to_vec = kern
				.entities
				.get(&to)
				.expect("inserted above")
				.vector
				.clone();
			let score = cosine(&from_vec, &to_vec);
			if score < SIMILARITY_EDGE_FLOOR {
				continue;
			}
			let rid = reason_id(&from, &to, ReasonKind::Similarity, "", "");
			let r = Reason {
				id: rid,
				from,
				to,
				kind: ReasonKind::Similarity,
				vector: average_vec(&from_vec, &to_vec),
				score,
				..Default::default()
			};
			if let Some(kern) = g.kerns.get_mut(root_id) {
				add_reason(kern, r);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bench_support::trace::{Trace, TraceDoc};

	#[test]
	fn build_graph_populates_a_searchable_lexical_index() {
		// Regression for the empty-lexical-index bug: a built graph's BM25 index must
		// return the token-overlapping doc, so "hybrid" trace queries actually use it
		// instead of silently falling back to dense-only.
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
