# src/types.rs — commentary

- Placement: `LlmFunc`/`EmbedFunc` live at the crate root — not inline at each call site — because they thread through many modules (`ingest::Worker`, `retrieval`, `capture_intake`, `tick`); one canonical definition keeps those signatures identical and importable without module-to-module type coupling.
- `EmbedFunc`: the `String` error is a known simplification — a structured error enum is a deferred follow-up; it would ripple through every embed producer and consumer.
