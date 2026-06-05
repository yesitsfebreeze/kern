use crate::ingest::outcome::FailureReport;
use crate::llm::{is_transient, Client as LlmClient};

pub(crate) async fn embed_chunks(
	embedder: &LlmClient,
	chunks: &[String],
) -> (Vec<Vec<f64>>, Vec<FailureReport>) {
	if chunks.is_empty() {
		return (Vec::new(), Vec::new());
	}

	if let Ok(vecs) = embedder.embed_batch(chunks).await {
		if vecs.len() == chunks.len() {
			return (vecs, Vec::new());
		}
	}

	let mut vecs = Vec::with_capacity(chunks.len());
	let mut failures = Vec::new();
	for (i, chunk) in chunks.iter().enumerate() {
		match embed_with_retry(embedder, chunk, "chunk", i).await {
			Ok(v) => vecs.push(v),
			Err(fail) => {
				failures.push(fail);
				vecs.push(Vec::new());
			}
		}
	}
	(vecs, failures)
}

pub(crate) async fn embed_with_retry(
	embedder: &LlmClient,
	text: &str,
	scope: &str,
	chunk_index: usize,
) -> Result<Vec<f64>, FailureReport> {
	let delays = [150, 300, 600];
	let mut last_err = None;

	for delay_ms in delays.iter() {
		match embedder.embed(text).await {
			Ok(v) => return Ok(v),
			Err(e) => {
				if !is_transient(&e) {
					return Err(FailureReport {
						scope: scope.into(),
						chunk_index,
						class: "permanent".into(),
						error: e.to_string(),
					});
				}
				last_err = Some(e);
				tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
			}
		}
	}

	Err(FailureReport {
		scope: scope.into(),
		chunk_index,
		class: "transient".into(),
		error: last_err.map(|e| e.to_string()).unwrap_or_default(),
	})
}
