# src/trnsprt/tests/integration.rs — commentary

- `registry_register_inproc_rejects_duplicate_server_id`: intentionally duplicates the registry.rs unit-test path — the unit test uses a local MockServer; this one exercises the guard through the integration crate's public `Registry` surface with `AdderServer`. Don't delete one as redundant.

Second-pass migration:

- `PER_CALL_CEILING` (5 ms) 4 lines -> 2. The measured reasoning, for the record: `InProcTransport` is a direct function call with no IO and no syscalls, so real per-call overhead should sit well under a millisecond. 5 ms is therefore a deliberately LOOSE regression ceiling — it is not a latency target and should not be tightened toward the real number. It exists to catch an accidental blocking call or a per-call allocation landing on the hot path. `LATENCY_ITERS` (100) averages out scheduler/timer noise; its one-line justification stayed inline.
- `inproc_call_tool_missing_argument_is_invalid_params`: deleted the 2-line body comment and the trailing `// missing "b"` data label — the test name states the contract, and the `json!({ "a": 1 })` literal shows the omission. The contract: `AdderServer` requires both operands, so omitting one surfaces a -32602 (Invalid params) `McpError::Rpc` through the in-process transport + client.
Design notes (moved from source comments during comment sweep):
- PER_CALL_CEILING is a loose ceiling — it catches an accidental blocking call or per-call allocation on the hot path, NOT real latency (the in-proc transport is a direct function call). LATENCY_ITERS = calls averaged in the latency probe to smooth scheduler/timer noise.
