//! Test-only MCP infrastructure ([`mcp_pipe`]), scoped by being a dev-dependency —
//! deliberately NOT `#![cfg(test)]`, which compiles to nothing for dependents' tests.

pub mod mcp_pipe;
