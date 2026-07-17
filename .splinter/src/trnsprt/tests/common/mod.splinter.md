# src/trnsprt/tests/common/mod.rs — commentary

Second-pass migration:

- Module doc 6 lines -> 2. The retained point is the "don't try to DRY this" oracle: only the codec/adapter boilerplate is shared here. Each typed-RPC test file keeps its own type-specific `spawn_mock_server` because the `service!`-generated clients (`SearchSvcClient`, `KernRpcClient`) share no common trait, so those wrappers cannot collapse into one generic helper.
- `channel_pair`'s "(client, server)" return order stayed inline — both halves have the same type, so nothing else distinguishes them.
Design notes (moved from source comments during comment sweep):
- Shared transport plumbing for the trnsprt integration tests. Each test keeps its own spawn_mock_server — the generated RPC clients share no trait, so spawn_mock_server can't move here.
- channel_pair returns a connected client/server Channel pair over an in-process adapter, both framed with JsonEnvelopeCodec, in (client, server) order.
