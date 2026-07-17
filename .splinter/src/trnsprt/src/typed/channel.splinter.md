# src/trnsprt/src/typed/channel.rs — commentary

- `Channel::new`: the second codec instance for the writer half comes from `C::default()`; both concrete codecs (`JsonEnvelopeCodec`, `BincodeCodec`) are zero-sized so it's free (also stated in the module doc — the `Default` bound exists for this).
- `send`: uses the `SinkExt` method form; the `use futures::SinkExt` import exists solely so `.send` resolves without the fully-qualified spelling.
- `recv_returns_none_on_closed_adapter` (test): dropping one side closes its write half; the peer's reader hits EOF and `recv` must surface a clean `Ok(None)`, not an error.

Second-pass migration: module doc compressed (Codec-mirrors-Encoder/Decoder bridge + why two codec instances). `adapter_err_from_codec` inline comment deleted — Framed{Read,Write} surface either a CodecError or an io::Error wrapped into the codec's Error type; our codecs use `CodecError` directly, so it folds straight into `AdapterError::Codec` (already in the note above).
Design notes (moved from source comments during comment sweep):
- Channel<C> pairs a transport's reader/writer halves with a Codec for framed send/recv, backed by tokio_util::codec::{FramedRead, FramedWrite}. Each half owns its own codec instance; the writer's comes from Codec::Default (both codecs are zero-sized, so free).
