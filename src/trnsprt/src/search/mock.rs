// Mock mirrors the trait's explicit `impl Future` surface.
#![allow(clippy::manual_async_fn)]
//! In-memory [`SearchSvc`] handler for tests. `cancel_token`: only the highest
//! token seen yields `fresh: true`; older in-flight requests report stale.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::dto::{
	EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, Facet, NeighborsReq, NeighborsRes,
	PreviewReq, PreviewRes, SearchReq, SearchRes,
};
use super::svc::SearchSvc;

/// Mock [`SearchSvc`]. State is `Arc`-shared, so all clones observe the same
/// cancel-token watermark.
#[derive(Clone, Default)]
pub struct MockSearchServer {
	inner: Arc<MockState>,
}

#[derive(Default)]
struct MockState {
	/// Highest `cancel_token` seen; atomic so concurrent calls bump monotonically.
	high_water: AtomicU64,
}

impl MockSearchServer {
	pub fn new() -> Self {
		Self::default()
	}

	/// Canned corpus shared by `search` and `neighbors`.
	fn corpus() -> [EntityRef; 4] {
		[
			EntityRef {
				id: "e:fact:1".into(),
				kind: EntityKindLite::Fact,
				status: EntityStatusLite::Active,
				scheme: "inline".into(),
				label: "Rust borrow checker rejects aliased mutable refs".into(),
				snippet: "&mut T is unique.".into(),
				score: 0.95,
				edges: vec![],
			},
			EntityRef {
				id: "e:doc:1".into(),
				kind: EntityKindLite::Document,
				status: EntityStatusLite::Active,
				scheme: "file".into(),
				label: "src/main.rs".into(),
				snippet: "fn main() { ... }".into(),
				score: 0.81,
				edges: vec![],
			},
			EntityRef {
				id: "e:q:1".into(),
				kind: EntityKindLite::Question,
				status: EntityStatusLite::Active,
				scheme: "ticket".into(),
				label: "Why does borrow checker block this?".into(),
				snippet: "T-101".into(),
				score: 0.72,
				edges: vec![],
			},
			EntityRef {
				id: "e:claim:1".into(),
				kind: EntityKindLite::Claim,
				status: EntityStatusLite::Superseded,
				scheme: "agent".into(),
				label: "Agents recommend using RefCell".into(),
				snippet: "(superseded)".into(),
				score: 0.30,
				edges: vec![],
			},
		]
	}

	/// Facets AND across the list; within each `Facet` the `scheme`/`kind`
	/// axes also AND when both are set.
	fn filter(query: &str, facets: &[Facet], k: u32) -> Vec<EntityRef> {
		let q = query.to_lowercase();
		let mut hits: Vec<EntityRef> = Self::corpus()
			.into_iter()
			.filter(|e| {
				facets.iter().all(|f| {
					f.kind.is_none_or(|k| k == e.kind) && f.scheme.as_ref().is_none_or(|s| s == &e.scheme)
				})
			})
			.filter(|e| {
				q.is_empty() || e.label.to_lowercase().contains(&q) || e.snippet.to_lowercase().contains(&q)
			})
			.collect();
		hits.sort_by(|a, b| {
			b.score
				.partial_cmp(&a.score)
				.unwrap_or(std::cmp::Ordering::Equal)
		});
		hits.truncate(k as usize);
		hits
	}
}

impl SearchSvc for MockSearchServer {
	fn search(&self, req: SearchReq) -> impl ::core::future::Future<Output = SearchRes> + Send {
		let state = self.inner.clone();
		async move {
			let token = req.cancel_token.unwrap_or(0);
			let prev = state.high_water.fetch_max(token, Ordering::SeqCst);
			let high = prev.max(token);
			let fresh = token >= high; // == when token==high; >= so absent tokens still fresh
			SearchRes {
				hits: Self::filter(&req.query, &req.facets, req.k.max(1)),
				fresh,
			}
		}
	}

	fn neighbors(
		&self,
		req: NeighborsReq,
	) -> impl ::core::future::Future<Output = NeighborsRes> + Send {
		async move {
			let _depth = req.depth.min(3);
			let neighbors: Vec<EntityRef> = Self::corpus()
				.into_iter()
				.filter(|e| e.id != req.entity_id)
				.collect();
			// Restricting kinds without `Supports` drops the Claim row — canned
			// demo of edge-kind filtering.
			let neighbors = if !req.edge_kinds.is_empty() && !req.edge_kinds.contains(&EdgeKind::Supports)
			{
				neighbors
					.into_iter()
					.filter(|e| !matches!(e.kind, EntityKindLite::Claim))
					.collect()
			} else {
				neighbors
			};
			NeighborsRes { neighbors }
		}
	}

	fn preview(&self, req: PreviewReq) -> impl ::core::future::Future<Output = PreviewRes> + Send {
		async move {
			match req.entity_id.as_str() {
				"e:doc:1" => PreviewRes::File {
					path: "src/main.rs".into(),
					content: "fn main() { println!(\"hi\"); }\n".into(),
					language: Some("rust".into()),
				},
				"e:edge:1" => PreviewRes::Edge {
					from_label: "Fact A".into(),
					to_label: "Conclusion B".into(),
					kind: EdgeKind::Supports,
					sentence: "Fact A supports Conclusion B.".into(),
				},
				_ => PreviewRes::Text {
					content: format!("entity {}: canned text body.", req.entity_id),
				},
			}
		}
	}

	fn kinds(&self) -> impl ::core::future::Future<Output = Vec<EntityKindLite>> + Send {
		async {
			vec![
				EntityKindLite::Fact,
				EntityKindLite::Claim,
				EntityKindLite::Document,
				EntityKindLite::Question,
				EntityKindLite::Answer,
				EntityKindLite::Conclusion,
				EntityKindLite::Superseded,
			]
		}
	}
}

#[cfg(test)]
mod facet_filter_tests {
	use super::*;

	fn req(facets: Vec<Facet>) -> SearchReq {
		SearchReq {
			query: String::new(),
			facets,
			k: 100,
			cancel_token: None,
		}
	}

	#[tokio::test]
	async fn empty_facets_return_full_corpus() {
		let svc = MockSearchServer::new();
		let res = svc.search(req(vec![])).await;
		assert_eq!(res.hits.len(), 4);
	}

	#[tokio::test]
	async fn kind_only_facet_keeps_only_matching_kind() {
		let svc = MockSearchServer::new();
		let res = svc
			.search(req(vec![Facet {
				kind: Some(EntityKindLite::Fact),
				scheme: None,
			}]))
			.await;
		assert!(!res.hits.is_empty());
		assert!(res.hits.iter().all(|h| h.kind == EntityKindLite::Fact));
	}

	#[tokio::test]
	async fn scheme_only_facet_keeps_only_matching_scheme() {
		let svc = MockSearchServer::new();
		let res = svc
			.search(req(vec![Facet {
				kind: None,
				scheme: Some("file".into()),
			}]))
			.await;
		assert!(!res.hits.is_empty());
		assert!(res.hits.iter().all(|h| h.scheme == "file"));
	}

	#[tokio::test]
	async fn kind_and_scheme_facet_intersect() {
		let svc = MockSearchServer::new();
		let res = svc
			.search(req(vec![Facet {
				kind: Some(EntityKindLite::Fact),
				scheme: Some("inline".into()),
			}]))
			.await;
		assert_eq!(res.hits.len(), 1);
		let h = &res.hits[0];
		assert_eq!(h.kind, EntityKindLite::Fact);
		assert_eq!(h.scheme, "inline");
	}

	#[tokio::test]
	async fn multiple_facets_are_anded() {
		let svc = MockSearchServer::new();
		let res = svc
			.search(req(vec![
				Facet {
					kind: Some(EntityKindLite::Fact),
					scheme: None,
				},
				Facet {
					kind: None,
					scheme: Some("inline".into()),
				},
			]))
			.await;
		assert_eq!(res.hits.len(), 1);
		assert_eq!(res.hits[0].kind, EntityKindLite::Fact);
		assert_eq!(res.hits[0].scheme, "inline");
	}

	#[tokio::test]
	async fn cancel_token_marks_the_older_request_stale() {
		let svc = MockSearchServer::new();
		let sreq = |tok| SearchReq {
			query: String::new(),
			facets: vec![],
			k: 5,
			cancel_token: Some(tok),
		};
		assert!(
			svc.search(sreq(2)).await.fresh,
			"token 2 is high-water -> fresh"
		);
		assert!(
			!svc.search(sreq(1)).await.fresh,
			"token 1 < high-water -> stale"
		);
		assert!(svc.search(sreq(3)).await.fresh, "token 3 re-bumps -> fresh");
	}

	#[tokio::test]
	async fn neighbors_edge_kind_filter_drops_claim_when_supports_excluded() {
		let svc = MockSearchServer::new();
		let nreq = |kinds| NeighborsReq {
			entity_id: "e:fact:1".into(),
			edge_kinds: kinds,
			depth: 1,
		};
		let all = svc.neighbors(nreq(vec![])).await;
		assert!(
			all
				.neighbors
				.iter()
				.any(|e| e.kind == EntityKindLite::Claim),
			"Claim present unfiltered"
		);
		let restricted = svc.neighbors(nreq(vec![EdgeKind::Contradicts])).await;
		assert!(
			restricted
				.neighbors
				.iter()
				.all(|e| e.kind != EntityKindLite::Claim),
			"Claim dropped"
		);
		let with_supports = svc.neighbors(nreq(vec![EdgeKind::Supports])).await;
		assert!(
			with_supports
				.neighbors
				.iter()
				.any(|e| e.kind == EntityKindLite::Claim),
			"Supports keeps Claim"
		);
	}

	#[tokio::test]
	async fn preview_dispatches_all_three_variants() {
		let svc = MockSearchServer::new();
		assert!(matches!(
			svc
				.preview(PreviewReq {
					entity_id: "e:doc:1".into()
				})
				.await,
			PreviewRes::File { .. }
		));
		assert!(matches!(
			svc
				.preview(PreviewReq {
					entity_id: "e:edge:1".into()
				})
				.await,
			PreviewRes::Edge { .. }
		));
		assert!(matches!(
			svc
				.preview(PreviewReq {
					entity_id: "e:other:9".into()
				})
				.await,
			PreviewRes::Text { .. }
		));
	}
}
