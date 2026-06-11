//! Shared helpers for the trnsprt integration tests.
//!
//! The `spawn_mock_server` in each typed-RPC test (search_rpc, kern_rpc) is
//! irreducibly type-specific — each builds a *different* generated client
//! (`SearchSvcClient` vs `KernRpcClient`) over a different mock and `serve_*`
//! fn, and the generated clients share no common trait, so the wrappers cannot
//! collapse into one. What IS identical across them is the transport plumbing:
//! an in-process adapter pair wired into two `JsonEnvelopeCodec` channels. That
//! boilerplate lives here so the two tests stop copy-pasting it.

use trnsprt::typed::{Channel, InprocAdapter, JsonEnvelopeCodec};

/// A connected client/server channel pair over an in-process adapter, both
/// framed with `JsonEnvelopeCodec`. Returned in `(client, server)` order.
pub fn channel_pair() -> (Channel<JsonEnvelopeCodec>, Channel<JsonEnvelopeCodec>) {
    let (client_side, server_side) = InprocAdapter::pair();
    (
        Channel::new(client_side, JsonEnvelopeCodec::new()),
        Channel::new(server_side, JsonEnvelopeCodec::new()),
    )
}
