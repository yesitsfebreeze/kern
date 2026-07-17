use std::sync::Arc;

// Infallible by convention: an outage arrives as "" — callers treat "" as "skip".
pub type LlmFunc = Arc<dyn Fn(&str) -> String + Send + Sync>;

pub type EmbedFunc = Arc<dyn Fn(&str) -> Result<Vec<f32>, String> + Send + Sync>;
