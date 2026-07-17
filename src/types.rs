//! Cross-cutting capability types: the boxed LLM and embedding closures.
//!
//! [`LlmFunc`] and [`EmbedFunc`] are the dependency-injection seam between kern's
//! pure graph / ingest / retrieval logic and the concrete `llm::Client`. Those
//! layers accept a closure rather than importing the client directly, so they
//! stay unit-testable with stub closures and carry no hard LLM dependency.
//!
//! The aliases live at the crate root — not inline at each call site — because
//! they thread through many modules (`ingest::Worker`, `retrieval`,
//! `intake`, `tick`); one canonical definition keeps those signatures
//! identical and importable without module-to-module type coupling.

use std::sync::Arc;

/// Text → completion. Infallible at this boundary: an outage is surfaced as an
/// empty string by the producing closure, which callers treat as "skip".
pub type LlmFunc = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Text → embedding vector, or an error message. The `String` error is a known
/// simplification (a structured error enum is a deferred follow-up; it would
/// ripple through every embed producer and consumer).
pub type EmbedFunc = Arc<dyn Fn(&str) -> Result<Vec<f32>, String> + Send + Sync>;
