//! Integration tests for `KernRpc`: end-to-end client/server roundtrip on
//! `InprocAdapter` + `JsonEnvelopeCodec`, plus the cancellation race on `query`.

use std::sync::Arc;

use trnsprt::kern_rpc::{
	EdgeKind, EntityKindLite, IngestReq, KernRpcClient, LinkReq, MockKernServer, NeighborsReq,
	QueryReq, SourceLite,
};
use trnsprt::typed::JsonEnvelopeCodec;

mod common;

fn spawn_mock_server() -> (
	KernRpcClient<JsonEnvelopeCodec>,
	tokio::task::JoinHandle<()>,
	Arc<MockKernServer>,
) {
	let (client_chan, server_chan) = common::channel_pair();
	let client = KernRpcClient::new(client_chan);
	let mock = Arc::new(MockKernServer::new());
	let mock_for_server = (*mock).clone();
	let handle = tokio::spawn(async move {
		let _ = trnsprt::kern_rpc::serve_kern_rpc(server_chan, mock_for_server).await;
	});
	(client, handle, mock)
}

fn query_req(text: &str, k: u32, cancel_token: Option<u64>) -> QueryReq {
	QueryReq {
		text: text.into(),
		k,
		cancel_token,
		..Default::default()
	}
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
		.query(query_req("borrow", 5, Some(1)))
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
async fn ingest_with_descriptor_succeeds() {
	let (client, handle, _mock) = spawn_mock_server();

	let res = client
		.ingest(IngestReq {
			text: "graph nodes carry confidence".into(),
			source: SourceLite::Inline {
				hash: "h2".into(),
				section: String::new(),
			},
			kind: EntityKindLite::Claim,
			descriptor: Some("provenance=test note=annotated".into()),
			conf: 0.7,
			sync: true,
		})
		.await
		.expect("ingest with descriptor");
	assert!(!res.entity_id.is_empty());
	assert_eq!(res.status, "ingested");

	handle.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_marks_older_keystroke_as_stale() {
	let (client, handle, _mock) = spawn_mock_server();

	let newer = client
		.query(query_req("rust", 5, Some(2)))
		.await
		.expect("newer");
	assert!(newer.fresh);

	let older = client
		.query(query_req("rust", 5, Some(1)))
		.await
		.expect("older");
	assert!(!older.fresh, "older keystroke must be reported stale");

	handle.abort();
}
