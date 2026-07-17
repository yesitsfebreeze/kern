# src/gossip/transport.rs — commentary

- Module split rationale: framing lives apart from `node.rs`'s networking policy (heartbeat / broadcast / forward) so the framing layer can evolve — and be tested over loopback — independently of peer-selection logic.


Second-pass migration:
- Test comment in `decode_rejects_frame_over_max_size` compressed to one line; contract: a length prefix of `GOSSIP_MAX_FRAME_BYTES + 1` must be rejected before any body allocation or read.
- Framing docs kept inline (wire contract: big-endian u32 length prefix + bincode body).
- `read_frame` doc trimmed to 2 lines to meet the length rule. The dropped clause: the oversize prefix is rejected before allocating OR READING the body — that ordering is the DoS guard (a hostile peer must not be able to make us reserve `u32::MAX` bytes on its say-so), and is pinned by `decode_rejects_frame_over_max_size`.
