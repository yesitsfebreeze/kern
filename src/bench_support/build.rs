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
	g
}

/// Insert every trace document into the root kern as a Claim entity carrying the
/// deterministic bench embedding of its text.
fn insert_docs(g: &mut GraphGnn, root_id: &str, trace: &Trace) {
	for doc in &trace.docs {
		let vec = embed::embed(&doc.text);
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
			kind: EntityKind::Claim,
			..Default::default()
		};
		if let Some(kern) = g.kerns.get_mut(root_id) {
			kern.entities.insert(t.id.clone(), t);
		}
	}
}

/// Seed similarity edges between every pair of documents whose cosine clears a
/// floor. O(n^2) on purpose: benchmark traces are small (tens to low-hundreds of
/// docs), so the full pairwise edge set is cheaper and more faithful than
/// approximating it via the ANN index. If trace corpora ever grow large, replace
/// this with a top-k batch index build.
fn seed_similarity_edges(g: &mut GraphGnn, root_id: &str, trace: &Trace) {
	let ids: Vec<String> = trace.docs.iter().map(|d| d.id.clone()).collect();
	for i in 0..ids.len() {
		for j in (i + 1)..ids.len() {
			let from = ids[i].clone();
			let to = ids[j].clone();
			let kern = g.kerns.get(root_id).expect("root kern exists");
			let from_vec = kern.entities.get(&from).expect("inserted above").vector.clone();
			let to_vec = kern.entities.get(&to).expect("inserted above").vector.clone();
			let score = cosine(&from_vec, &to_vec);
			if score < 0.1 {
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
