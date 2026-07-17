# src/test-utils/src/lib.rs — commentary

Convention: add future shared test helpers (in-memory transports, fake daemon handles, ...) as sibling modules of `mcp_pipe` in this crate, so consumers reach everything under one `test_utils::*` path.
Second-pass migration (module doc compressed to 2 lines):
- Full `cfg(test)` explanation: `#![cfg(test)]` makes a crate compile *nothing for its dependents*, which would break integration tests in other crates that consume `mcp_pipe` — e.g. `trnsprt/tests/integration.rs` imports `AdderServer` from here. The crate is scoped to tests by being a dev-dependency, never linked into a production binary.
- Usage sketch from another crate's `tests/`: `let (mut transport, handle) = new_pipe(); handle.push_reply(&reply_result(1, json!({"ok": true})));` then drive an MCP client against `transport` and assert on `handle.drain_frames()` to inspect what the client sent.
