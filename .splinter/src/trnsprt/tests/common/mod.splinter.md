# src/trnsprt/tests/common/mod.rs — commentary

Second-pass migration:

- Module doc 6 lines -> 2. The retained point is the "don't try to DRY this" oracle: only the codec/adapter boilerplate is shared here. Each typed-RPC test file keeps its own type-specific `spawn_mock_server` because the `service!`-generated clients (`SearchSvcClient`, `KernRpcClient`) share no common trait, so those wrappers cannot collapse into one generic helper.
- `channel_pair`'s "(client, server)" return order stayed inline — both halves have the same type, so nothing else distinguishes them.
