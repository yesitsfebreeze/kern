// Mock returns explicit `impl Future` to mirror the trait surface; async-fn
// rewrite adds no value in a test double.
#![allow(clippy::manual_async_fn)]
//! In-memory [`KernRpc`] handler for tests and downstream slice
//! development.
//!
//! The mock keeps a tiny in-memory store of `EntityRef`s plus a list of
//! `Reason` edges. `query` does a substring scan over labels, `ingest`
//! appends a fresh row, `link` records an edge, `neighbors` returns
//! every other entity in the corpus filtered by edge kind, and
//! `truncate_after` clears entries newer than the supplied timestamp.
//!
//! Honours `cancel_token` semantics on `query`: only the highest token
//! seen yields `fresh: true`. Older in-flight requests come back with
//! `fresh: false` so palette frames can suppress stale frames.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::dto::{
    CallToolReq, CallToolRes, DegradeReq, DegradeRes, DescriptorReq, DescriptorRes, EdgeKind,
    EntityKindLite, EntityRef, EntityStatusLite, ForgetReq, ForgetRes, HealthRes, IngestReq,
    AnchorReq, AnchorRes, IngestRes, LinkReq, LinkRes, NeighborsReq, NeighborsRes, PulseReq,
    PulseRes, QueryReq, QueryRes, TruncateAfterReq, TruncateAfterRes,
};
use super::svc::KernRpc;

#[derive(Clone, Debug)]
struct MockEntity {
    pub r#ref: EntityRef,
    pub ts_ms: u64,
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

    /// Seed the mock with one hit so `query` returns something useful
    /// without requiring an `ingest` first. Used by integration tests.
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
            ts_ms: 0,
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
            // Parse the optional `kind` (lower-case label) into the lite
            // enum so we can compare on equal terms with `EntityRef.kind`.
            // An unrecognised string disables the kind filter rather than
            // silently dropping every hit.
            let kind_filter = EntityKindLite::from_label(&req.kind);
            let scheme_filter = if req.source.is_empty() {
                None
            } else {
                Some(req.source.as_str())
            };
            let g = state.entities.lock().unwrap();
            // Apply facet filters BEFORE the substring label match so the
            // result count is bounded by the most specific predicate
            // first. AND across (kind, scheme) — both must hold when
            // either is set.
            let mut hits: Vec<EntityRef> = g
                .iter()
                .filter(|e| kind_filter.is_none_or(|k| k == e.r#ref.kind))
                .filter(|e| scheme_filter.is_none_or(|s| s == e.r#ref.scheme))
                .filter(|e| q.is_empty() || e.r#ref.label.to_lowercase().contains(&q))
                .map(|e| e.r#ref.clone())
                .collect();
            hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
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
                ts_ms: now_ms(),
            };
            // Touch `req.descriptor`/`req.conf`/`req.source` to silence
            // unused warnings when DTO fields don't drive the mock.
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
            // `text` isn't stored in the mock — assert it via a debug
            // attribute instead so callers can still log it.
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
            // `depth` is clamped but NOT traversed: the mock only ever returns
            // direct (depth-1) neighbours regardless of the requested depth. The
            // clamp documents the server's intended ceiling; multi-hop expansion
            // is intentionally out of scope for the test double (see the
            // `neighbors_returns_only_direct_edges_regardless_of_depth` test).
            let _depth = req.depth.min(3);
            let entities = state.entities.lock().unwrap();
            let edges = state.edges.lock().unwrap();
            // Index entities by id once so the per-edge endpoint lookup is O(1)
            // instead of an O(n) linear scan — keeps `neighbors` near-linear in
            // edge count even on a large seeded corpus.
            let by_id: std::collections::HashMap<&str, &EntityRef> =
                entities.iter().map(|e| (e.r#ref.id.as_str(), &e.r#ref)).collect();
            let allowed = |k: EdgeKind| {
                req.edge_kinds.is_empty() || req.edge_kinds.contains(&k)
            };
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

    fn truncate_after(
        &self,
        req: TruncateAfterReq,
    ) -> impl ::core::future::Future<Output = TruncateAfterRes> + Send {
        let state = self.inner.clone();
        async move {
            state
                .entities
                .lock()
                .unwrap()
                .retain(|e| e.ts_ms <= req.ts_ms);
            TruncateAfterRes {}
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
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod facet_filter_tests {
    use super::*;
    use crate::kern_rpc::IngestReq;
    use crate::kern_rpc::dto::SourceLite;

    /// Seed a mixed corpus of N=4 entities spanning two kinds and two
    /// schemes so both filter axes are exercised.
    async fn seeded() -> MockKernServer {
        let mock = MockKernServer::new();
        // Fact + file scheme.
        mock.query_ingest("fact-file alpha", EntityKindLite::Fact, "file").await;
        // Fact + inline scheme.
        mock.query_ingest("fact-inline beta", EntityKindLite::Fact, "inline").await;
        // Claim + file scheme.
        mock.query_ingest("claim-file gamma", EntityKindLite::Claim, "file").await;
        // Claim + inline scheme.
        mock.query_ingest("claim-inline delta", EntityKindLite::Claim, "inline").await;
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
        // Guards against silently eating every hit when callers pass a
        // typo'd kind label — the filter must degrade to "no kind
        // constraint" rather than "match nothing".
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
        // Documents the mock's depth behaviour: `depth` is clamped to 3 but never
        // traversed — only direct (depth-1) neighbours come back. A deeper
        // request does NOT pull transitive nodes.
        use crate::kern_rpc::{EdgeKind, LinkReq, NeighborsReq};
        let mock = MockKernServer::new();
        let a = mock.seed("a", EntityKindLite::Claim);
        let b = mock.seed("b", EntityKindLite::Claim);
        let c = mock.seed("c", EntityKindLite::Claim);
        // a -> b -> c chain.
        let _ = mock
            .link(LinkReq { from_id: a.clone(), to_id: b.clone(), reason_kind: EdgeKind::Supports, text: String::new() })
            .await;
        let _ = mock
            .link(LinkReq { from_id: b.clone(), to_id: c.clone(), reason_kind: EdgeKind::Supports, text: String::new() })
            .await;

        let res = mock
            .neighbors(NeighborsReq { entity_id: a.clone(), edge_kinds: vec![], depth: 3 })
            .await;
        let ids: Vec<&str> = res.neighbors.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec![b.as_str()], "depth-1 only: direct neighbour b");
        assert!(!ids.contains(&c.as_str()), "transitive c must NOT be reached");
    }

    #[test]
    fn from_label_maps_content_kinds_and_rejects_superseded() {
        assert_eq!(EntityKindLite::from_label("fact"), Some(EntityKindLite::Fact));
        assert_eq!(EntityKindLite::from_label("conclusion"), Some(EntityKindLite::Conclusion));
        // Superseded is a status, not a kind -> None (degrades to "no filter").
        assert_eq!(EntityKindLite::from_label("superseded"), None);
        assert_eq!(EntityKindLite::from_label("bogus"), None);
        assert_eq!(EntityKindLite::from_label(""), None);
    }
}
