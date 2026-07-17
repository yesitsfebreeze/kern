# src/trnsprt/tests/kern_rpc.rs — commentary

The old module doc also claimed "DTO bincode + JSON serde roundtrips" coverage — stale; those live in `kern_rpc::dto`'s unit tests, not here. Deleted.

- `ingest_with_descriptor_succeeds`: exists to exercise the non-None `descriptor` branch on `IngestReq` through the wire — the field must serialize, deserialize, and be accepted by the server.

Second-pass migration:

- Module doc 3 lines -> 2, dropping the aside that `query`'s cancellation race mirrors `SearchSvc::search` semantics (the `search_rpc.rs` note already records that shared contract).
- `query_req` helper: doc deleted. It built a `QueryReq` from text/k/cancel_token and left mode/answer/kind/source at their `Default` (empty), collapsing per-call empty-string boilerplate — evident from the `..Default::default()` body.
- Kept inline: the `// depth clamping: any value over 3 should still answer.` line in `link_then_neighbors_walks_the_edge`, which is the only thing explaining the magic `depth: 99`.
