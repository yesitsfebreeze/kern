# src/trnsprt/src/typed/codec.rs — commentary

The module doc used to claim a blanket impl converts any `Codec` into tokio_util's `Encoder`/`Decoder` — false (orphan rule); each concrete codec carries explicit delegating impls instead. Deleted as stale.

- `BincodeCodec`: could parameterise on the frame type instead of `Vec<u8>`, but Phase 1 only needed the JSON codec; this sits as a placeholder for hot-path hops.
- Oversize handling is deliberately symmetric: encode refuses frames past `MAX_FRAME_LEN` (nothing written), decode fails fast on a header claiming more — we never emit a frame we'd later reject, and a bogus length never stalls the reader into buffering gigabytes.

Second-pass migration: `JsonEnvelopeCodec` doc trimmed — decode scans for the next `\n`; the envelope shape (`{ id, method, params }` / `{ id, result|error }`) is the caller's. `MAX_FRAME_LEN` doc compressed to the DoS trap (symmetric encode/decode rejection already noted above). Test-shim comment (`enc`/`dec`) trimmed: both `Codec` and tokio_util's `Encoder`/`Decoder` are in scope, so plain `encode`/`decode` calls are ambiguous — the `Codec`-bound shims disambiguate.
Design notes (moved from source comments during comment sweep):
- Codec = the wire format. API mirrors tokio_util::codec: encode writes one frame into dst, decode tries to extract one frame from src.
- No blanket impl<T: Codec> Encoder<T::Frame> for T (orphan-rule grief) — each concrete codec carries delegating Encoder/Decoder impls. (KEPT in source)
- JsonEnvelopeCodec: line-delimited JSON envelope; each frame is a Value's JSON text + '\n'. The envelope shape is the caller's — this codec only shuttles Values. (wire framing KEPT tightened in source). Its decode loops (NOT recursion) over leading blank lines to avoid a stack overflow on N consecutive newlines (see json_many_consecutive_newlines_do_not_overflow regression test).
- BincodeCodec: length-delimited bincode (bincode 2 standard config); wire = 4-byte big-endian u32 length prefix then payload; frames are Vec<u8>, caller bincode-encodes their typed envelope first. (byte layout KEPT tightened in source)
- MAX_FRAME_LEN 64 MiB cap: without it a bogus 4-byte header claiming ~4 GiB makes the reader buffer that much before yielding (OOM/DoS); encode rejects oversized frames too. (KEPT in source). decode rejects an oversized length before reserving the buffer (fail fast).
- Test shims enc/dec exist because both Codec and tokio_util's Encoder/Decoder are in scope, making the method calls ambiguous.
