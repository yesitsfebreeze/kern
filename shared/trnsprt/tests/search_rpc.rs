//! Integration tests for `SearchSvc`.
//!
//! Covers
//! - end-to-end client/server roundtrip on `InprocAdapter` +
//!   `JsonEnvelopeCodec` (the typed-RPC stack hardcodes its frame type
//!   to `serde_json::Value`; bincode-serializability of the DTOs is
//!   exercised in unit tests inside `search::dto`),
//! - cancellation race: a newer search supersedes an older one — the
//!   stale response surfaces with `fresh: false`.

use std::sync::Arc;

use trnsprt::search::{
    EdgeKind, EntityKindLite, Facet, MockSearchServer, NeighborsReq, PreviewReq, PreviewRes,
    SearchReq, SearchSvcClient,
};
use trnsprt::typed::JsonEnvelopeCodec;

mod common;

fn spawn_mock_server() -> (
    SearchSvcClient<JsonEnvelopeCodec>,
    tokio::task::JoinHandle<()>,
    Arc<MockSearchServer>,
) {
    let (client_chan, server_chan) = common::channel_pair();
    let client = SearchSvcClient::new(client_chan);
    let mock = Arc::new(MockSearchServer::new());
    let mock_for_server = (*mock).clone();
    let handle = tokio::spawn(async move {
        let _ = trnsprt::search::serve_search_svc(server_chan, mock_for_server).await;
    });
    (client, handle, mock)
}

#[tokio::test(flavor = "multi_thread")]
async fn search_roundtrips_filtered_hits() {
    let (client, handle, _mock) = spawn_mock_server();

    let res = client
        .search(SearchReq {
            query: "borrow".into(),
            facets: vec![Facet { scheme: None, kind: Some(EntityKindLite::Fact) }],
            k: 10,
            cancel_token: Some(1),
        })
        .await
        .expect("search rpc");

    assert!(!res.hits.is_empty(), "expected at least one Fact hit");
    assert!(res.hits.iter().all(|h| h.kind == EntityKindLite::Fact));
    assert!(res.fresh, "first request must be fresh");

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn neighbors_respects_edge_kind_filter() {
    let (client, handle, _mock) = spawn_mock_server();

    // Empty edge_kinds = all edges; depth gets clamped to 3 server-side.
    let all = client
        .neighbors(NeighborsReq {
            entity_id: "e:fact:1".into(),
            edge_kinds: vec![],
            depth: 99,
        })
        .await
        .expect("neighbors rpc");
    assert!(all.neighbors.iter().any(|e| e.kind == EntityKindLite::Claim));

    // Restricting to References excludes the Claim-class neighbour.
    let restricted = client
        .neighbors(NeighborsReq {
            entity_id: "e:fact:1".into(),
            edge_kinds: vec![EdgeKind::References],
            depth: 1,
        })
        .await
        .expect("neighbors rpc");
    assert!(restricted.neighbors.iter().all(|e| e.kind != EntityKindLite::Claim));

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_returns_file_variant_for_document() {
    let (client, handle, _mock) = spawn_mock_server();

    let res = client
        .preview(PreviewReq { entity_id: "e:doc:1".into() })
        .await
        .expect("preview rpc");
    match res {
        PreviewRes::File { path, language, .. } => {
            assert_eq!(path, "src/main.rs");
            assert_eq!(language.as_deref(), Some("rust"));
        }
        other => panic!("expected File variant, got {other:?}"),
    }

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_returns_non_file_variants_for_non_document_entities() {
    let (client, handle, _mock) = spawn_mock_server();

    // An edge entity -> Edge variant (not File).
    match client.preview(PreviewReq { entity_id: "e:edge:1".into() }).await.expect("preview rpc") {
        PreviewRes::Edge { .. } => {}
        other => panic!("expected Edge variant, got {other:?}"),
    }
    // Any other non-document entity -> generic Text body.
    match client.preview(PreviewReq { entity_id: "e:claim:1".into() }).await.expect("preview rpc") {
        PreviewRes::Text { content } => assert!(!content.is_empty(), "Text body present"),
        other => panic!("expected Text variant, got {other:?}"),
    }

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn search_with_unmatched_scheme_returns_no_hits_not_an_error() {
    let (client, handle, _mock) = spawn_mock_server();

    // The scheme axis is an exact-match filter, so a scheme that no corpus
    // entity carries yields an empty result set — but the RPC must still
    // SUCCEED (graceful, fresh), not surface an error or panic.
    let res = client
        .search(SearchReq {
            query: String::new(),
            facets: vec![Facet { scheme: Some("bogus".into()), kind: None }],
            k: 10,
            cancel_token: None,
        })
        .await
        .expect("search must succeed on an unknown scheme");
    assert!(res.hits.is_empty(), "unmatched scheme -> no hits");
    assert!(res.fresh, "still a fresh response");

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn kinds_returns_all_seven_canonical_variants() {
    let (client, handle, _mock) = spawn_mock_server();

    let kinds = client.kinds().await.expect("kinds rpc");
    assert_eq!(kinds.len(), 7);
    assert!(kinds.contains(&EntityKindLite::Fact));
    assert!(kinds.contains(&EntityKindLite::Conclusion));

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_marks_older_keystroke_as_stale() {
    // Issue search(token=2) first; it bumps the high-water mark to 2.
    // A subsequent search(token=1) is older and must come back stale.
    let (client, handle, _mock) = spawn_mock_server();

    let newer = client
        .search(SearchReq {
            query: "rust".into(),
            facets: vec![],
            k: 5,
            cancel_token: Some(2),
        })
        .await
        .expect("newer search");
    assert!(newer.fresh, "high-water request must be fresh");

    let older = client
        .search(SearchReq {
            query: "rust".into(),
            facets: vec![],
            k: 5,
            cancel_token: Some(1),
        })
        .await
        .expect("older search");
    assert!(!older.fresh, "older keystroke must be reported stale");

    // A still-newer request bumps the watermark again and is fresh.
    let newest = client
        .search(SearchReq {
            query: "rust".into(),
            facets: vec![],
            k: 5,
            cancel_token: Some(3),
        })
        .await
        .expect("newest search");
    assert!(newest.fresh);

    handle.abort();
}
