# src/trnsprt/src/error.rs — commentary

Second-pass migration:

- Kept inline only the `Protocol` vs `Rpc` distinction (both directions): `Protocol` is a wire-format violation, `Rpc` is a *well-formed* JSON-RPC error reply — application-level, not a transport fault. That pairing is the one trap here; the arm names alone don't convey it.
- Deleted as restatement of the arm name + its `#[error(...)]` string: `Io`, `Json`, `UnknownServer`, `DuplicateServer`, `NotRunning` doc lines.
- Deleted the `is_transient` doc ("true only for connection-level faults; every other arm is deterministic") — the `matches!` body lists the two arms literally, and the test `is_transient_is_true_only_for_connection_level_faults` states the contract in its name.
- `NotRunning` is connection-level: a supervisor may respawn the child and retry. `Rpc.code` follows JSON-RPC conventions (-32601 method-not-found, -32602 invalid-params).
Design notes (moved from source comments during comment sweep):
- McpError::Protocol is a wire-format violation — distinct from McpError::Rpc, which is a *well-formed* error response.
- McpError::Rpc is a well-formed JSON-RPC error reply — application-level, NOT a transport fault. Its `code` follows JSON-RPC conventions (-32601, -32602, ...).
