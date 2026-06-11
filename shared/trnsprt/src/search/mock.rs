// Mock returns explicit `impl Future` to mirror the trait surface; async-fn
// rewrite adds no value in a test double.
#![allow(clippy::manual_async_fn)]
//! In-memory [`SearchSvc`] handler for tests and downstream slice
//! development (palette UI, preview pane).
//!
//! Returns canned hits and previews from a small, hand-curated corpus.
//! Honours `cancel_token` semantics: only the **highest** token seen so
//! far yields a `fresh: true` response — every older in-flight request
//! is reported as stale. Production kern wiring may use the same flag
//! to suppress out-of-order frame application in the palette.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::dto::{
    EdgeKind, EntityKindLite, EntityRef, EntityStatusLite, Facet, NeighborsReq, NeighborsRes,
    PreviewReq, PreviewRes, SearchReq, SearchRes,
};
use super::svc::SearchSvc;

/// Mock implementation of [`SearchSvc`]. Cheap to clone — internal
/// state is `Arc`-shared, so multiple handles observe the same
/// cancel-token watermark.
#[derive(Clone, Default)]
pub struct MockSearchServer {
    inner: Arc<MockState>,
}

#[derive(Default)]
struct MockState {
    /// Highest `cancel_token` seen across all `search` calls so far.
    /// Atomic so concurrent calls update it monotonically.
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

    /// Filter the canned corpus by facets + free-text substring match.
    /// Trivial — production code would invoke kern's fused index.
    ///
    /// Facets are AND-ed across the list; within each `Facet` the
    /// `scheme` and `kind` axes are also AND-ed when both are set. The
    /// facet predicate runs BEFORE the substring scan so the result set
    /// is bounded by the most specific filter first — matters for
    /// downstream tests that assert facet semantics on small corpora.
    fn filter(query: &str, facets: &[Facet], k: u32) -> Vec<EntityRef> {
        let q = query.to_lowercase();
        let mut hits: Vec<EntityRef> = Self::corpus()
            .into_iter()
            .filter(|e| {
                facets.iter().all(|f| {
                    f.kind.is_none_or(|k| k == e.kind)
                        && f.scheme.as_ref().is_none_or(|s| s == &e.scheme)
                })
            })
            .filter(|e| {
                q.is_empty()
                    || e.label.to_lowercase().contains(&q)
                    || e.snippet.to_lowercase().contains(&q)
            })
            .collect();
        // Highest score first — mirrors fused-rank order.
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(k as usize);
        hits
    }
}

impl SearchSvc for MockSearchServer {
    fn search(&self, req: SearchReq) -> impl ::core::future::Future<Output = SearchRes> + Send {
        let state = self.inner.clone();
        async move {
            // Update high-water mark; treat absent token as 0.
            let token = req.cancel_token.unwrap_or(0);
            let prev = state.high_water.fetch_max(token, Ordering::SeqCst);
            // After fetch_max, the stored value is max(prev, token).
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
            // Return everything from the corpus that isn't `req.entity_id`.
            // Filter by edge_kinds is a no-op in the mock unless caller
            // explicitly asked for `Supports` only — exercised by tests.
            let neighbors: Vec<EntityRef> = Self::corpus()
                .into_iter()
                .filter(|e| e.id != req.entity_id)
                .collect();
            // If caller restricted edge kinds and didn't include
            // `Supports`, drop the Claim-class result to demonstrate
            // filtering behaviour.
            let neighbors = if !req.edge_kinds.is_empty()
                && !req.edge_kinds.contains(&EdgeKind::Supports)
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
        // Fact+inline matches the canned `e:fact:1` row only.
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
        // First facet narrows by kind, second by scheme — intersection
        // must still return only the Fact+inline row.
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

    // ── cancellation / freshness ────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_token_marks_the_older_request_stale() {
        let svc = MockSearchServer::new();
        let sreq = |tok| SearchReq { query: String::new(), facets: vec![], k: 5, cancel_token: Some(tok) };
        // The higher token bumps the high-water mark and is fresh.
        assert!(svc.search(sreq(2)).await.fresh, "token 2 is high-water -> fresh");
        // A lower token issued after is stale (1 < high-water 2).
        assert!(!svc.search(sreq(1)).await.fresh, "token 1 < high-water -> stale");
        // A still-higher token bumps again and is fresh.
        assert!(svc.search(sreq(3)).await.fresh, "token 3 re-bumps -> fresh");
    }

    // ── neighbors edge-kind filter ──────────────────────────────────────────

    #[tokio::test]
    async fn neighbors_edge_kind_filter_drops_claim_when_supports_excluded() {
        let svc = MockSearchServer::new();
        let nreq = |kinds| NeighborsReq { entity_id: "e:fact:1".into(), edge_kinds: kinds, depth: 1 };
        // No restriction -> the Claim-class neighbour is present.
        let all = svc.neighbors(nreq(vec![])).await;
        assert!(all.neighbors.iter().any(|e| e.kind == EntityKindLite::Claim), "Claim present unfiltered");
        // Restrict to a non-Supports kind -> the Claim row is dropped.
        let restricted = svc.neighbors(nreq(vec![EdgeKind::Contradicts])).await;
        assert!(restricted.neighbors.iter().all(|e| e.kind != EntityKindLite::Claim), "Claim dropped");
        // Explicitly including Supports keeps it.
        let with_supports = svc.neighbors(nreq(vec![EdgeKind::Supports])).await;
        assert!(with_supports.neighbors.iter().any(|e| e.kind == EntityKindLite::Claim), "Supports keeps Claim");
    }

    // ── preview variant dispatch ────────────────────────────────────────────

    #[tokio::test]
    async fn preview_dispatches_all_three_variants() {
        // Guards against a future entity_id arm silently regressing a variant.
        let svc = MockSearchServer::new();
        assert!(matches!(
            svc.preview(PreviewReq { entity_id: "e:doc:1".into() }).await,
            PreviewRes::File { .. }
        ));
        assert!(matches!(
            svc.preview(PreviewReq { entity_id: "e:edge:1".into() }).await,
            PreviewRes::Edge { .. }
        ));
        assert!(matches!(
            svc.preview(PreviewReq { entity_id: "e:other:9".into() }).await,
            PreviewRes::Text { .. }
        ));
    }
}
