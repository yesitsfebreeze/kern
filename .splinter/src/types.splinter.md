# src/types.rs — commentary

- Placement: `LlmFunc`/`EmbedFunc` live at the crate root — not inline at each call site — because they thread through many modules (`ingest::Worker`, `retrieval`, `capture_intake`, `tick`); one canonical definition keeps those signatures identical and importable without module-to-module type coupling.
- `EmbedFunc`: the `String` error is a known simplification — a structured error enum is a deferred follow-up; it would ripple through every embed producer and consumer.
## Design context (moved from source doc comments)

- Module: cross-cutting capability types — the boxed LLM and embedding closures. This is the dependency-injection seam between pure graph logic and `llm::Client`.
- `LlmFunc` = `Arc<dyn Fn(&str) -> String>`: text -> completion. Infallible by convention — an outage arrives as an empty string, which callers treat as "skip". (A concise form of this invariant is kept in source since the type signature alone doesn't convey it.)
- `EmbedFunc` = `Arc<dyn Fn(&str) -> Result<Vec<f32>, String>>`: text -> embedding vector, or an error message.
