# src/trnsprt/src/typed/error.rs — commentary

The three-way split (Adapter/Codec/Rpc) follows the typed-RPC design doc. These types landed alongside the legacy `McpError`; Phase 2 may remove `McpError` — for now both coexist.

- `From<AdapterError>/From<CodecError> for RpcError`: collapse lower transport/wire errors into the application-layer `RpcError` as strings, since `RpcError` is the boundary the generated stubs return — lets `service!`-generated client/serve code use `?` instead of repeating `.map_err(|e| RpcError::Adapter(e.to_string()))` at every call site.
- `io_error_into_codec_is_a_decode_carrying_the_original_message` (test): guards `From<io::Error>` against silently truncating/altering the message text.

Second-pass migration: module doc compressed to the one-per-layer split. `From<io::Error> for CodecError` comment trimmed — `Channel` wraps the decode-side failure back into `AdapterError::Codec`.
