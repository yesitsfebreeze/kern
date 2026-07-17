# src/trnsprt/src/kern_rpc/client_local.rs — commentary

- `jittered`: why full-jitter — when several clients race on `kern --daemon` startup, a fixed delay makes them all retry in lockstep (thundering herd hitting the listener at the same instants); a per-attempt random offset desynchronises them.
- `connect_endpoint_gives_up_after_exhausting_retries` (test): there is no port file — the endpoint itself is the coordination — so dialing a bogus endpoint exercises the real retry path without standing up a server.

Second-pass migration — detail moved out of doc comments:

- Module doc: the endpoint is fixed per cwd (see `Endpoint::kern`); the only coordination kern and its clients need is agreeing on the resolver.
- `RETRIES` (5) / `RETRY_DELAY_MS` (100) are public so callers/tests can reference the baseline budget.
- `connect_endpoint`: useful for tests that spawn kern at a private path/pipe name.
- `connect_endpoint_with_retry`: exposed so a high-latency CI environment or a test can widen/shrink the budget without patching the constants.
