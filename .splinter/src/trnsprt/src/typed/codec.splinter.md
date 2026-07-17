# src/trnsprt/src/typed/codec.rs — commentary

The module doc used to claim a blanket impl converts any `Codec` into tokio_util's `Encoder`/`Decoder` — false (orphan rule); each concrete codec carries explicit delegating impls instead. Deleted as stale.

- `BincodeCodec`: could parameterise on the frame type instead of `Vec<u8>`, but Phase 1 only needed the JSON codec; this sits as a placeholder for hot-path hops.
- Oversize handling is deliberately symmetric: encode refuses frames past `MAX_FRAME_LEN` (nothing written), decode fails fast on a header claiming more — we never emit a frame we'd later reject, and a bogus length never stalls the reader into buffering gigabytes.

Second-pass migration: `JsonEnvelopeCodec` doc trimmed — decode scans for the next `\n`; the envelope shape (`{ id, method, params }` / `{ id, result|error }`) is the caller's. `MAX_FRAME_LEN` doc compressed to the DoS trap (symmetric encode/decode rejection already noted above). Test-shim comment (`enc`/`dec`) trimmed: both `Codec` and tokio_util's `Encoder`/`Decoder` are in scope, so plain `encode`/`decode` calls are ambiguous — the `Codec`-bound shims disambiguate.
