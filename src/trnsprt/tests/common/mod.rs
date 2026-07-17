use trnsprt::typed::{Channel, InprocAdapter, JsonEnvelopeCodec};

// Returned in `(client, server)` order.
pub fn channel_pair() -> (Channel<JsonEnvelopeCodec>, Channel<JsonEnvelopeCodec>) {
	let (client_side, server_side) = InprocAdapter::pair();
	(
		Channel::new(client_side, JsonEnvelopeCodec::new()),
		Channel::new(server_side, JsonEnvelopeCodec::new()),
	)
}
