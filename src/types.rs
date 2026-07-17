//! Cross-cutting capability types: the boxed LLM and embedding closures — the
//! dependency-injection seam between pure graph logic and `llm::Client`.

use std::sync::Arc;

/// Text → completion. Infallible here: an outage arrives as an empty string,
/// which callers treat as "skip".
pub type LlmFunc = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// Text → embedding vector, or an error message.
pub type EmbedFunc = Arc<dyn Fn(&str) -> Result<Vec<f32>, String> + Send + Sync>;
