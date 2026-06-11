//! Shared **test-only** infrastructure for the kern workspace.
//!
//! This crate exists purely to back integration tests across the workspace — it
//! is consumed as a `dev-dependency` and is never linked into a production
//! binary. Today it provides [`mcp_pipe`]: an in-memory MCP transport pipe plus a
//! sample `AdderServer`, so a test can drive an MCP client/server round-trip with
//! no sockets or subprocesses.
//!
//! Typical use from another crate's `tests/`:
//! ```ignore
//! use test_utils::mcp_pipe::{new_pipe, reply_result};
//! let (mut transport, handle) = new_pipe();
//! handle.push_reply(&reply_result(1, serde_json::json!({ "ok": true })));
//! // ...drive an MCP client against `transport`, then assert on
//! //    `handle.drain_frames()` to inspect what the client sent.
//! ```
//!
//! Add future shared helpers (in-memory transports, fake daemon handles, ...) as
//! sibling modules here, so consumers reach them under one `test_utils::*` path.
//!
//! NB: this is deliberately NOT `#![cfg(test)]`. That attribute makes a crate
//! compile nothing for its *dependents*, which would break the integration tests
//! in other crates that consume `mcp_pipe` (e.g. `trnsprt/tests/integration.rs`
//! imports `AdderServer` from here). The crate is scoped to tests by being a
//! dev-dependency, not by a `cfg`.

pub mod mcp_pipe;
