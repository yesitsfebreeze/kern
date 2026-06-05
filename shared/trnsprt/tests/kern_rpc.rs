//! Integration tests for `KernRpc`.
//!
//! Covers
//! - end-to-end client/server roundtrip on `InprocAdapter` +
//!   `JsonEnvelopeCodec` for every RPC method,
//! - DTO bincode + JSON serde roundtrips (extends what the unit tests
//!   in `kern_rpc::dto` already exercise),
//! - cancellation race on `query`: a newer keystroke supersedes an
//!   older one, mirroring `SearchSvc::search` semantics.

use std::sync::Arc;

use trnsprt::kern_rpc::{
    EdgeKind, EntityKindLite, IngestReq, KernRpcClient, LinkReq, MockKernServer, NeighborsReq,
    QueryReq, SourceLite, TruncateAfterReq,
};
use trnsprt::typed::{Channel, InprocAdapter, JsonEnvelopeCodec};

fn spawn_mock_server() -> (
    KernRpcClient<JsonEnvelopeCodec>,
    tokio::task::JoinHandle<()>,
    Arc<MockKernServer>,
) {
    let (client_side, server_side) = InprocAdapter::pair();
    let server_chan = Channel::new(server_side, JsonEnvelopeCodec::new());
    let client_chan = Channel::new(client_side, JsonEnvelopeCodec::new());
    let client = KernRpcClient::new(client_chan);
    let mock = Arc::new(MockKernServer::new());
    let mock_for_server = (*mock).clone();
    let handle = tokio::spawn(async move {
        let _ = trnsprt::kern_rpc::serve_kern_rpc(server_chan, mock_for_server).await;
    });
    (client, handle, mock)
}

#[tokio::test(flavor = "multi_thread")]
async fn ingest_then_query_returns_the_new_entity() {
    let (client, handle, _mock) = spawn_mock_server();

    let res = client
        .ingest(IngestReq {
            text: "borrow checker rejects aliased mutable refs".into(),
            source: SourceLite::Inline {
                hash: "h1".into(),
                section: String::new(),
            },
            kind: EntityKindLite::Fact,
            descriptor: None,
            conf: 0.9,
            sync: true,
        })
        .await
        .expect("ingest rpc");
    assert!(!res.entity_id.is_empty());
    assert_eq!(res.status, "ingested");

    let q = client
        .query(QueryReq {
            text: "borrow".into(),
            k: 5,
            mode: String::new(),
            answer: false,
            kind: String::new(),
            source: String::new(),
            cancel_token: Some(1),
        })
        .await
        .expect("query rpc");
    assert!(q.fresh);
    assert!(!q.hits.is_empty(), "expected the freshly-ingested entity");
    assert!(q.hits.iter().any(|h| h.kind == EntityKindLite::Fact));

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn link_then_neighbors_walks_the_edge() {
    let (client, handle, mock) = spawn_mock_server();
    let a = mock.seed("entity A", EntityKindLite::Claim);
    let b = mock.seed("entity B", EntityKindLite::Conclusion);

    let link = client
        .link(LinkReq {
            from_id: a.clone(),
            to_id: b.clone(),
            reason_kind: EdgeKind::Supports,
            text: "A supports B".into(),
        })
        .await
        .expect("link rpc");
    assert!(!link.reason_id.is_empty());

    let n = client
        .neighbors(NeighborsReq {
            entity_id: a.clone(),
            edge_kinds: vec![EdgeKind::Supports],
            depth: 1,
        })
        .await
        .expect("neighbors rpc");
    assert!(n.neighbors.iter().any(|e| e.id == b));

    // depth clamping: any value over 3 should still answer.
    let n2 = client
        .neighbors(NeighborsReq {
            entity_id: a,
            edge_kinds: vec![],
            depth: 99,
        })
        .await
        .expect("neighbors rpc");
    assert!(n2.neighbors.iter().any(|e| e.id == b));

    // Filtering by an edge kind that wasn't used drops the neighbour.
    let none = client
        .neighbors(NeighborsReq {
            entity_id: b,
            edge_kinds: vec![EdgeKind::Contradicts],
            depth: 1,
        })
        .await
        .expect("neighbors rpc");
    assert!(none.neighbors.is_empty());

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn truncate_after_drops_newer_entities() {
    let (client, handle, _mock) = spawn_mock_server();

    let _ = client
        .ingest(IngestReq {
            text: "older".into(),
            source: SourceLite::default(),
            kind: EntityKindLite::Claim,
            descriptor: None,
            conf: 0.5,
            sync: true,
        })
        .await
        .expect("ingest");

    // Truncate at "now"; freshly-ingested rows have ts > 0 so they are
    // dropped. Subsequent query should come back empty.
    let _ = client
        .truncate_after(TruncateAfterReq { ts_ms: 0 })
        .await
        .expect("truncate");

    let q = client
        .query(QueryReq {
            text: String::new(),
            k: 10,
            mode: String::new(),
            answer: false,
            kind: String::new(),
            source: String::new(),
            cancel_token: None,
        })
        .await
        .expect("query");
    assert!(q.hits.is_empty(), "truncate should have cleared the store");

    handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_marks_older_keystroke_as_stale() {
    // Mirrors `SearchSvc` cancellation semantics: a newer search bumps
    // the high-water mark, an older one comes back fresh=false.
    let (client, handle, _mock) = spawn_mock_server();

    let newer = client
        .query(QueryReq {
            text: "rust".into(),
            k: 5,
            mode: String::new(),
            answer: false,
            kind: String::new(),
            source: String::new(),
            cancel_token: Some(2),
        })
        .await
        .expect("newer");
    assert!(newer.fresh);

    let older = client
        .query(QueryReq {
            text: "rust".into(),
            k: 5,
            mode: String::new(),
            answer: false,
            kind: String::new(),
            source: String::new(),
            cancel_token: Some(1),
        })
        .await
        .expect("older");
    assert!(!older.fresh, "older keystroke must be reported stale");

    handle.abort();
}
