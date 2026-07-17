//! Shared transport plumbing for the trnsprt integration tests. Each test keeps
//! its own `spawn_mock_server` — generated clients share no trait, so it can't move here.

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
