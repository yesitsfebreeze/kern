# splinter: src/test-utils/src/mcp_pipe.rs

Second-pass migration:
- `ClientEnd` asymmetry detail (compressed inline to 2 lines): `PipeTransport` holds two `ClientEnd`s over the SAME `Wire` and uses only each one's matching half — `reader` for `Read` (drains `from_server`, server → client), `writer` for `Write` (appends `to_server`, client → server). The split is purely which trait method gets called; a `ClientEnd` driven through the opposite trait would silently touch the wrong buffer.
- `AdderServer::call_tool` strict params: both operands required, integers only; a missing/non-integer arg is `-32602 Invalid params` rather than a silent default-to-zero so callers can exercise the argument-validation error path.
