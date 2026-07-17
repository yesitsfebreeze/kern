// Mock mirrors the trait's explicit `impl Future` surface.
#![allow(clippy::manual_async_fn)]
//! In-memory [`KernRpc`] handler for tests. `query` honours `cancel_token`:
//! only the highest token seen yields `fresh: true`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::dto::{
	AnchorReq, AnchorRes, CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq,
	DescriptorRes, EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, ForgetReq, ForgetRes,
	HealthRes, IngestReq, IngestRes, LinkReq, LinkRes, ListToolsReq, ListToolsRes, NeighborsReq,
	NeighborsRes, PulseReq, PulseRes, QueryReq, QueryRes,
};
use super::svc::KernRpc;

#[derive(Clone, Debug)]
struct MockEntity {
	pub r#ref: EntityRef,
}

#[derive(Clone, Debug)]
struct MockEdge {
	pub from: String,
	pub to: String,
	pub kind: EdgeKind,
}

#[derive(Default)]
struct MockState {
	entities: Mutex<Vec<MockEntity>>,
	edges: Mutex<Vec<MockEdge>>,
	next_id: AtomicU64,
	high_water: AtomicU64,
}

#[derive(Clone, Default)]
pub struct MockKernServer {
	inner: Arc<MockState>,
}

impl MockKernServer {
	pub fn new() -> Self {
		Self::default()
	}

	fn next_id(&self, prefix: &str) -> String {
		let n = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
		format!("mock:{prefix}:{n}")
	}

	/// Seed one hit so `query` returns something without a prior `ingest`.
	pub fn seed(&self, label: &str, kind: EntityKindLite) -> String {
		let id = self.next_id("seed");
		let mut g = self.inner.entities.lock().unwrap();
		g.push(MockEntity {
			r#ref: EntityRef {
				id: id.clone(),
				kind,
				status: EntityStatusLite::Active,
				scheme: "inline".into(),
				label: label.into(),
				snippet: label.into(),
				score: 1.0,
				edges: vec![],
			},
		});
		id
	}
}

impl KernRpc for MockKernServer {
	fn query(&self, req: QueryReq) -> impl ::core::future::Future<Output = QueryRes> + Send {
		let state = self.inner.clone();
		async move {
			let token = req.cancel_token.unwrap_or(0);
			let prev = state.high_water.fetch_max(token, Ordering::SeqCst);
			let high = prev.max(token);
			let fresh = token >= high;
			let q = req.text.to_lowercase();
			// Unrecognised `kind` disables the filter, never "match nothing"
			// (see unrecognised_kind_string_disables_kind_filter).
			let kind_filter = EntityKindLite::from_label(&req.kind);
			let scheme_filter = if req.source.is_empty() {
				None
			} else {
				Some(req.source.as_str())
			};
			let g = state.entities.lock().unwrap();
			let mut hits: Vec<EntityRef> = g
				.iter()
				.filter(|e| kind_filter.is_none_or(|k| k == e.r#ref.kind))
				.filter(|e| scheme_filter.is_none_or(|s| s == e.r#ref.scheme))
				.filter(|e| q.is_empty() || e.r#ref.label.to_lowercase().contains(&q))
				.map(|e| e.r#ref.clone())
				.collect();
			hits.sort_by(|a, b| {
				b.score
					.partial_cmp(&a.score)
					.unwrap_or(std::cmp::Ordering::Equal)
			});
			hits.truncate(req.k.max(1) as usize);
			QueryRes {
				hits,
				answer: String::new(),
				fresh,
			}
		}
	}

	fn ingest(&self, req: IngestReq) -> impl ::core::future::Future<Output = IngestRes> + Send {
		let state = self.inner.clone();
		let next_id = self.next_id("ent");
		async move {
			let scheme = req.source.scheme().to_string();
			let label = if req.text.len() > 64 {
				format!("{}…", &req.text[..63])
			} else {
				req.text.clone()
			};
			let snippet = label.clone();
			let entity = MockEntity {
				r#ref: EntityRef {
					id: next_id.clone(),
					kind: req.kind,
					status: EntityStatusLite::Active,
					scheme,
					label,
					snippet,
					score: 1.0,
					edges: vec![],
				},
			};
			// Silence unused warnings for DTO fields the mock ignores.
			let _ = (&req.descriptor, req.conf, &req.source);
			state.entities.lock().unwrap().push(entity);
			IngestRes {
				entity_id: next_id,
				status: "ingested".into(),
				message: String::new(),
			}
		}
	}

	fn link(&self, req: LinkReq) -> impl ::core::future::Future<Output = LinkRes> + Send {
		let state = self.inner.clone();
		let next_id = self.next_id("edge");
		async move {
			let edge = MockEdge {
				from: req.from_id,
				to: req.to_id,
				kind: req.reason_kind,
			};
			// `text` isn't stored in the mock.
			let _ = req.text;
			state.edges.lock().unwrap().push(edge);
			LinkRes { reason_id: next_id }
		}
	}

	fn neighbors(
		&self,
		req: NeighborsReq,
	) -> impl ::core::future::Future<Output = NeighborsRes> + Send {
		let state = self.inner.clone();
		async move {
			// `depth` is clamped but NOT traversed — the mock is depth-1 only.
			let _depth = req.depth.min(3);
			let entities = state.entities.lock().unwrap();
			let edges = state.edges.lock().unwrap();
			let by_id: std::collections::HashMap<&str, &EntityRef> = entities
				.iter()
				.map(|e| (e.r#ref.id.as_str(), &e.r#ref))
				.collect();
			let allowed = |k: EdgeKind| req.edge_kinds.is_empty() || req.edge_kinds.contains(&k);
			let mut out = Vec::new();
			for edge in edges.iter() {
				if !allowed(edge.kind) {
					continue;
				}
				let other = if edge.from == req.entity_id {
					edge.to.as_str()
				} else if edge.to == req.entity_id {
					edge.from.as_str()
				} else {
					continue;
				};
				if let Some(r) = by_id.get(other) {
					out.push((*r).clone());
				}
			}
			NeighborsRes { neighbors: out }
		}
	}

	fn forget(&self, _req: ForgetReq) -> impl ::core::future::Future<Output = ForgetRes> + Send {
		async move { ForgetRes::default() }
	}

	fn degrade(&self, _req: DegradeReq) -> impl ::core::future::Future<Output = DegradeRes> + Send {
		async move { DegradeRes::default() }
	}

	fn health(&self) -> impl ::core::future::Future<Output = HealthRes> + Send {
		async move { HealthRes::default() }
	}

	fn anchor(&self, _req: AnchorReq) -> impl ::core::future::Future<Output = AnchorRes> + Send {
		async move { AnchorRes::default() }
	}

	fn descriptor(
		&self,
		_req: DescriptorReq,
	) -> impl ::core::future::Future<Output = DescriptorRes> + Send {
		async move { DescriptorRes::default() }
	}

	fn pulse(&self, _req: PulseReq) -> impl ::core::future::Future<Output = PulseRes> + Send {
		async move { PulseRes::default() }
	}

	fn call_tool(
		&self,
		_req: CallToolReq,
	) -> impl ::core::future::Future<Output = CallToolRes> + Send {
		async move { CallToolRes::default() }
	}

	fn list_tools(
		&self,
		_req: ListToolsReq,
	) -> impl ::core::future::Future<Output = ListToolsRes> + Send {
		async move { ListToolsRes::default() }
	}
}

#[cfg(test)]
mod facet_filter_tests {
	use super::*;
	use crate::kern_rpc::dto::SourceLite;
	use crate::kern_rpc::IngestReq;

	/// 4 entities = 2 kinds x 2 schemes, so both filter axes are exercised.
	async fn seeded() -> MockKernServer {
		let mock = MockKernServer::new();
		mock
			.query_ingest("fact-file alpha", EntityKindLite::Fact, "file")
			.await;
		mock
			.query_ingest("fact-inline beta", EntityKindLite::Fact, "inline")
			.await;
		mock
			.query_ingest("claim-file gamma", EntityKindLite::Claim, "file")
			.await;
		mock
			.query_ingest("claim-inline delta", EntityKindLite::Claim, "inline")
			.await;
		mock
	}

	impl MockKernServer {
		async fn query_ingest(&self, text: &str, kind: EntityKindLite, scheme: &str) {
			let source = match scheme {
				"file" => SourceLite::File {
					path: "x".into(),
					section: String::new(),
					title: String::new(),
					author: String::new(),
					url: String::new(),
				},
				"inline" => SourceLite::Inline {
					hash: "h".into(),
					section: String::new(),
				},
				other => panic!("unsupported test scheme {other}"),
			};
			let _ = self
				.ingest(IngestReq {
					text: text.into(),
					source,
					kind,
					descriptor: None,
					conf: 1.0,
					sync: true,
				})
				.await;
		}
	}

	fn req(kind: &str, source: &str) -> QueryReq {
		QueryReq {
			text: String::new(),
			k: 100,
			mode: String::new(),
			answer: false,
			kind: kind.into(),
			source: source.into(),
			cancel_token: None,
		}
	}

	#[tokio::test]
	async fn empty_filters_return_full_corpus() {
		let mock = seeded().await;
		let res = mock.query(req("", "")).await;
		assert_eq!(res.hits.len(), 4);
	}

	#[tokio::test]
	async fn kind_filter_only_returns_matching_kind() {
		let mock = seeded().await;
		let res = mock.query(req("fact", "")).await;
		assert_eq!(res.hits.len(), 2);
		assert!(res.hits.iter().all(|h| h.kind == EntityKindLite::Fact));
	}

	#[tokio::test]
	async fn scheme_filter_only_returns_matching_scheme() {
		let mock = seeded().await;
		let res = mock.query(req("", "file")).await;
		assert_eq!(res.hits.len(), 2);
		assert!(res.hits.iter().all(|h| h.scheme == "file"));
	}

	#[tokio::test]
	async fn kind_and_scheme_filters_intersect() {
		let mock = seeded().await;
		let res = mock.query(req("fact", "file")).await;
		assert_eq!(res.hits.len(), 1);
		let h = &res.hits[0];
		assert_eq!(h.kind, EntityKindLite::Fact);
		assert_eq!(h.scheme, "file");
	}

	#[tokio::test]
	async fn unrecognised_kind_string_disables_kind_filter() {
		let mock = seeded().await;
		let res = mock.query(req("notakind", "")).await;
		assert_eq!(res.hits.len(), 4);
	}

	#[tokio::test]
	async fn substring_filter_still_applies_after_facets() {
		let mock = seeded().await;
		let mut q = req("fact", "");
		q.text = "alpha".into();
		let res = mock.query(q).await;
		assert_eq!(res.hits.len(), 1);
		assert!(res.hits[0].label.contains("alpha"));
	}

	#[tokio::test]
	async fn neighbors_returns_only_direct_edges_regardless_of_depth() {
		use crate::kern_rpc::{EdgeKind, LinkReq, NeighborsReq};
		let mock = MockKernServer::new();
		let a = mock.seed("a", EntityKindLite::Claim);
		let b = mock.seed("b", EntityKindLite::Claim);
		let c = mock.seed("c", EntityKindLite::Claim);
		let _ = mock
			.link(LinkReq {
				from_id: a.clone(),
				to_id: b.clone(),
				reason_kind: EdgeKind::Supports,
				text: String::new(),
			})
			.await;
		let _ = mock
			.link(LinkReq {
				from_id: b.clone(),
				to_id: c.clone(),
				reason_kind: EdgeKind::Supports,
				text: String::new(),
			})
			.await;

		let res = mock
			.neighbors(NeighborsReq {
				entity_id: a.clone(),
				edge_kinds: vec![],
				depth: 3,
			})
			.await;
		let ids: Vec<&str> = res.neighbors.iter().map(|e| e.id.as_str()).collect();
		assert_eq!(ids, vec![b.as_str()], "depth-1 only: direct neighbour b");
		assert!(
			!ids.contains(&c.as_str()),
			"transitive c must NOT be reached"
		);
	}

	#[test]
	fn from_label_maps_content_kinds_and_rejects_superseded() {
		assert_eq!(
			EntityKindLite::from_label("fact"),
			Some(EntityKindLite::Fact)
		);
		assert_eq!(
			EntityKindLite::from_label("conclusion"),
			Some(EntityKindLite::Conclusion)
		);
		// Superseded is a status, not a kind -> None (degrades to "no filter").
		assert_eq!(EntityKindLite::from_label("superseded"), None);
		assert_eq!(EntityKindLite::from_label("bogus"), None);
		assert_eq!(EntityKindLite::from_label(""), None);
	}
}
