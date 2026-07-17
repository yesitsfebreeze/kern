# src/trnsprt/src/kern_rpc/client_local.rs — commentary

- `jittered`: why full-jitter — when several clients race on `kern --daemon` startup, a fixed delay makes them all retry in lockstep (thundering herd hitting the listener at the same instants); a per-attempt random offset desynchronises them.
- `connect_endpoint_gives_up_after_exhausting_retries` (test): there is no port file — the endpoint itself is the coordination — so dialing a bogus endpoint exercises the real retry path without standing up a server.

Second-pass migration — detail moved out of doc comments:

- Module doc: the endpoint is fixed per cwd (see `Endpoint::kern`); the only coordination kern and its clients need is agreeing on the resolver.
- `RETRIES` (5) / `RETRY_DELAY_MS` (100) are public so callers/tests can reference the baseline budget.
- `connect_endpoint`: useful for tests that spawn kern at a private path/pipe name.
- `connect_endpoint_with_retry`: exposed so a high-latency CI environment or a test can widen/shrink the budget without patching the constants.
Design notes (moved from source comments during comment sweep):
- KernRpcClient::connect_local dials the per-cwd kern endpoint with the JSON-envelope codec. No port file: the endpoint resolver IS the coordination.
- RETRIES = connect attempts before giving up — absorbs the daemon-start race (a client launched alongside `kern --daemon` may dial before the listener is up).
- RETRY_DELAY_MS = default base delay between connect attempts, in milliseconds (jittered at use). The MS unit is in the const name.
- connect_local connects to a kern singleton at the per-cwd endpoint; caller is expected to run on a tokio runtime. connect_endpoint uses the default retry budget (RETRIES / RETRY_DELAY_MS); connect_endpoint_with_retry retries up to `retries` times with a jittered base_delay.
- jittered(): full-jitter into [base/2, base]; zero base stays zero. Entropy is wall-clock sub-second nanos — deliberately no `rand` dependency.
- test bogus_endpoint() is an endpoint nothing is listening on, for exercising the failure path.
