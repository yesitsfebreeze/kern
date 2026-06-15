use crate::base::graph::GraphGnn;
use crate::config::RetrievalConfig;
use crate::retrieval::seed::Mode;

use super::embed;
use super::ndcg;
use super::trace::{Trace, TraceQuery};

#[derive(Debug, Clone)]
pub struct QueryReport {
	pub id: String,
	pub mode: Mode,
	pub ranked_ids: Vec<String>,
	pub expected_ids: Vec<String>,
	pub ndcg10: f64,
	/// Recall@10: coverage of the expected ids in the top-10, order-insensitive.
	pub recall10: f64,
}

#[derive(Debug, Clone)]
pub struct ReplayReport {
	pub trace_name: String,
	pub per_query: Vec<QueryReport>,
	pub mean_ndcg10: f64,
	pub mean_recall10: f64,
}

pub fn replay(g: &GraphGnn, cfg: &RetrievalConfig, trace: &Trace) -> ReplayReport {
	let mut per_query = Vec::with_capacity(trace.queries.len());
	let mut ndcg_sum = 0.0;
	let mut recall_sum = 0.0;
	for q in &trace.queries {
		let rep = run_one(g, cfg, q);
		ndcg_sum += rep.ndcg10;
		recall_sum += rep.recall10;
		per_query.push(rep);
	}
	let n = per_query.len() as f64;
	let (mean_ndcg10, mean_recall10) = if per_query.is_empty() {
		(0.0, 0.0)
	} else {
		(ndcg_sum / n, recall_sum / n)
	};
	ReplayReport {
		trace_name: trace.name.clone(),
		per_query,
		mean_ndcg10,
		mean_recall10,
	}
}

fn run_one(g: &GraphGnn, cfg: &RetrievalConfig, q: &TraceQuery) -> QueryReport {
	let mode = Mode::parse(&q.mode);
	let qvec = embed::embed(&q.query);
	// An optional kind filter makes the bench run the FILTERED retrieval path
	// (post-filtering today; the place to A/B filtered-ANN wiring). An unparseable
	// kind falls back to no filter rather than silently scoring the wrong thing.
	let opts = q
		.filter_kind
		.as_deref()
		.and_then(crate::base::types::EntityKind::parse)
		.map(|kind| crate::retrieval::score::QueryOptions {
			kind: Some(kind),
			..Default::default()
		});
	let result = crate::retrieval::answer::query(g, cfg, &qvec, &q.query, mode, None, None, opts);
	let ranked: Vec<String> = result
		.entities
		.iter()
		.map(|st| st.entity.id.clone())
		.collect();
	let ndcg10 = ndcg::ndcg_at_k(&ranked, &q.expected_ids, 10);
	let recall10 = ndcg::recall_at_k(&ranked, &q.expected_ids, 10);
	QueryReport {
		id: q.id.clone(),
		mode,
		ranked_ids: ranked,
		expected_ids: q.expected_ids.clone(),
		ndcg10,
		recall10,
	}
}

#[cfg(test)]
mod tests {
	use super::super::build::build_graph;
	use super::super::trace::{TraceDoc, TraceQuery};
	use super::*;

	fn doc(id: &str, text: &str) -> TraceDoc {
		TraceDoc {
			id: id.into(),
			text: text.into(),
			kind: None,
		}
	}

	fn doc_kind(id: &str, text: &str, kind: &str) -> TraceDoc {
		TraceDoc {
			id: id.into(),
			text: text.into(),
			kind: Some(kind.into()),
		}
	}

	/// End-to-end: build a graph from a tiny trace, replay a query whose text
	/// matches one doc, and assert that doc is retrieved with positive nDCG.
	/// Uses the deterministic bench embedder, so no LLM/network needed. We assert
	/// recall + positive ranking quality rather than an exact rank-1, because the
	/// full retrieval pipeline (graph expansion, MMR, GNN blend) reorders results.
	#[test]
	fn replay_retrieves_relevant_doc_with_positive_ndcg() {
		let trace = Trace {
			name: "fixture".into(),
			docs: vec![
				doc("d1", "rust ownership and the borrow checker"),
				doc("d2", "graph neural network message passing"),
				doc("d3", "vector cosine similarity nearest neighbour"),
			],
			queries: vec![TraceQuery {
				id: "q1".into(),
				query: "rust ownership and the borrow checker".into(),
				expected_ids: vec!["d1".into()],
				mode: "semantic".into(),
				filter_kind: None,
			}],
		};

		let g = build_graph(&trace);
		let report = replay(&g, &RetrievalConfig::default(), &trace);

		assert_eq!(report.per_query.len(), 1);
		assert!(
			report.per_query[0].ranked_ids.iter().any(|id| id == "d1"),
			"the relevant doc must appear in the ranked results, got {:?}",
			report.per_query[0].ranked_ids
		);
		assert!(
			report.mean_ndcg10 > 0.0,
			"expected positive ranking quality, got {}",
			report.mean_ndcg10
		);
		// The relevant doc is among only 3 results, so it is within the top-10 ->
		// full recall. (Coverage assertion, distinct from the ordering NDCG asserts.)
		assert_eq!(
			report.mean_recall10, 1.0,
			"the single expected doc is retrieved within k -> recall@10 = 1.0"
		);
		assert_eq!(
			report.per_query[0].recall10, 1.0,
			"per-query recall is populated"
		);
	}

	#[test]
	fn replay_applies_the_kind_filter_end_to_end() {
		// build_graph inserts every doc as a Claim, so a `fact` filter matches
		// NOTHING: the filtered query must score recall@10 = 0, proving the filter
		// runs through the full retrieve -> post-filter path. The same query with no
		// filter (or kind=claim) retrieves the relevant doc, confirming it is the
		// filter — not a broken query — that zeroed recall.
		let docs = vec![
			doc("d1", "rust ownership and the borrow checker"),
			doc("d2", "graph neural network message passing"),
		];
		let mk = |filter: Option<&str>| Trace {
			name: "filtered".into(),
			docs: docs.clone(),
			queries: vec![TraceQuery {
				id: "q1".into(),
				query: "rust ownership and the borrow checker".into(),
				expected_ids: vec!["d1".into()],
				mode: "semantic".into(),
				filter_kind: filter.map(str::to_string),
			}],
		};
		let g = build_graph(&mk(None));
		let cfg = RetrievalConfig::default();

		assert_eq!(
			replay(&g, &cfg, &mk(None)).mean_recall10,
			1.0,
			"unfiltered finds the doc"
		);
		assert_eq!(
			replay(&g, &cfg, &mk(Some("fact"))).mean_recall10,
			0.0,
			"a fact filter on a Claim-only graph zeroes recall -> filter applied end-to-end"
		);
		assert_eq!(
			replay(&g, &cfg, &mk(Some("claim"))).mean_recall10,
			1.0,
			"kind=claim matches -> recall restored, so it was the filter, not the query"
		);
	}

	#[test]
	fn filtered_query_recovers_a_minority_kind_buried_by_the_majority() {
		// 15 Claims + 2 Facts share identical text, so all are equally relevant by
		// vector and lexical score. The expected docs are the 2 Facts. With ties
		// broken by id ascending, every "c*" Claim sorts before the "fact*" docs,
		// burying both Facts past the top-10 -> an UNFILTERED query scores
		// recall@10 = 0. A kind=fact filter seeds only Facts (dense + importance +
		// lexical all filter at source), so both are retrieved -> recall@10 = 1.0.
		// End-to-end proof of the filtered-seed win on a fewer-than-k scenario.
		let text = "rust ownership and the borrow checker semantics";
		let mut docs: Vec<TraceDoc> = (0..15).map(|i| doc(&format!("c{i:02}"), text)).collect();
		docs.push(doc_kind("fact0", text, "fact"));
		docs.push(doc_kind("fact1", text, "fact"));

		let mk = |filter: Option<&str>| Trace {
			name: "buried-minority".into(),
			docs: docs.clone(),
			queries: vec![TraceQuery {
				id: "q".into(),
				query: text.into(),
				expected_ids: vec!["fact0".into(), "fact1".into()],
				mode: "hybrid".into(),
				filter_kind: filter.map(str::to_string),
			}],
		};
		let g = build_graph(&mk(None));
		let cfg = RetrievalConfig::default();

		let unfiltered = replay(&g, &cfg, &mk(None)).mean_recall10;
		let filtered = replay(&g, &cfg, &mk(Some("fact"))).mean_recall10;

		assert_eq!(filtered, 1.0, "kind=fact surfaces both buried Facts");
		assert!(
			filtered > unfiltered,
			"filtering recovers recall the unfiltered query loses to the majority kind \
			 (filtered {filtered} vs unfiltered {unfiltered})"
		);
	}

	#[test]
	fn filtered_query_survives_delivery_pool_truncation() {
		// Like the buried-minority test but with 60 Claims (> the ~50 delivery cap),
		// so filter_delivery truncates BEFORE the filter could run. Identical text
		// connects every doc, so expansion floods the pool with non-matching Claims.
		// If the filter is applied only after truncation, the id-trailing Facts get
		// cut and recall@10 collapses. A correct order keeps recall@10 = 1.0.
		let text = "rust ownership and the borrow checker semantics".to_string();
		let mut docs: Vec<TraceDoc> = (0..60).map(|i| doc(&format!("c{i:03}"), &text)).collect();
		docs.push(doc_kind("fact0", &text, "fact"));
		docs.push(doc_kind("fact1", &text, "fact"));
		let trace = Trace {
			name: "big-buried".into(),
			docs,
			queries: vec![TraceQuery {
				id: "q".into(),
				query: text,
				expected_ids: vec!["fact0".into(), "fact1".into()],
				mode: "hybrid".into(),
				filter_kind: Some("fact".into()),
			}],
		};
		let g = build_graph(&trace);
		let recall = replay(&g, &RetrievalConfig::default(), &trace).mean_recall10;
		assert_eq!(
			recall, 1.0,
			"Facts must survive pool truncation under an active filter"
		);
	}

	#[test]
	fn replay_of_empty_trace_is_zero_mean() {
		let trace = Trace {
			name: "empty".into(),
			docs: vec![],
			queries: vec![],
		};
		let g = build_graph(&trace);
		let report = replay(&g, &RetrievalConfig::default(), &trace);
		assert_eq!(report.mean_ndcg10, 0.0, "no queries -> zero mean, not NaN");
		assert!(report.per_query.is_empty());
	}
}
