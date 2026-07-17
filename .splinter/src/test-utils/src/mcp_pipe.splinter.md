# splinter: src/test-utils/src/mcp_pipe.rs

Second-pass migration:
- `ClientEnd` asymmetry detail (compressed inline to 2 lines): `PipeTransport` holds two `ClientEnd`s over the SAME `Wire` and uses only each one's matching half — `reader` for `Read` (drains `from_server`, server → client), `writer` for `Write` (appends `to_server`, client → server). The split is purely which trait method gets called; a `ClientEnd` driven through the opposite trait would silently touch the wrong buffer.
- `AdderServer::call_tool` strict params: both operands required, integers only; a missing/non-integer arg is `-32602 Invalid params` rather than a silent default-to-zero so callers can exercise the argument-validation error path.
## Design context (moved from source doc comments)

- `ClientEnd` is one client end of the in-memory pipe. Directional: `Read` drains `from_server`, `Write` appends to `to_server` — drive each end only through its matching trait.
- `AdderServer::call_tool`: a missing or non-integer arg is a -32602 Rpc error, never a silent default-to-zero — tests rely on exercising this validation path.
