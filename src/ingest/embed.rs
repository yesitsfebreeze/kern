use crate::ingest::outcome::FailureReport;
use crate::llm::{is_transient, Client as LlmClient};

/// Backoff schedule for transient embed failures: up to 3 attempts, sleeping
/// 150ms, then 300ms, then 600ms between them. A *permanent* error (per
/// [`is_transient`]) skips the remaining attempts and fails immediately.
const RETRY_DELAYS_MS: [u64; 3] = [150, 300, 600];

/// Embed `chunks`. Fast path: one batch call when it returns exactly one vector
/// per chunk. Otherwise (batch error or length mismatch) fall back to embedding
/// each chunk individually with [`embed_with_retry`]. Returns the embeddings —
/// an empty `Vec` in the slot of any chunk that ultimately failed — paired with a
/// [`FailureReport`] per failed chunk. An empty input short-circuits to empties.
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

/// Embed `text`, retrying transient errors on the [`RETRY_DELAYS_MS`] backoff
/// schedule. A permanent error bails immediately as a `class: "permanent"`
/// [`FailureReport`]; exhausting every retry yields `class: "transient"`.
/// `scope`/`chunk_index` are echoed into the report for caller context.
pub(crate) async fn embed_with_retry(
	embedder: &LlmClient,
	text: &str,
	scope: &str,
	chunk_index: usize,
) -> Result<Vec<f64>, FailureReport> {
	let mut last_err = None;

	for delay_ms in RETRY_DELAYS_MS.iter() {
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

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::{json, Value};

	/// `/api/embed` stub returning exactly one embedding per input string — the
	/// "honest" server, so the batch path matches counts and short-circuits.
	fn echo_count_app() -> axum::Router {
		axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|body: axum::Json<Value>| async move {
				let n = body
					.0
					.get("input")
					.and_then(|v| v.as_array())
					.map(|a| a.len())
					.unwrap_or(1);
				let embs: Vec<Vec<f64>> = (0..n).map(|_| vec![0.1, 0.2, 0.3]).collect();
				axum::Json(json!({ "embeddings": embs }))
			}),
		)
	}

	#[tokio::test]
	async fn embed_chunks_short_circuits_on_the_batch_path_when_counts_match() {
		let (url, _server) = crate::test_support::spawn_http(echo_count_app()).await;
		let client = LlmClient::new_embed_only(&url, "m");
		let (vecs, fails) = embed_chunks(&client, &["a".into(), "b".into()]).await;
		assert_eq!(vecs.len(), 2);
		assert!(
			vecs.iter().all(|v| !v.is_empty()),
			"both embedded via the batch call"
		);
		assert!(fails.is_empty());
	}

	#[tokio::test]
	async fn embed_chunks_falls_back_to_per_item_on_a_count_mismatch() {
		// Always returns ONE embedding regardless of input count, so a 2-input batch
		// mismatches and drops to the per-item loop (each single input then succeeds).
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|| async { axum::Json(json!({ "embeddings": [[0.5, 0.6]] })) }),
		);
		let (url, _server) = crate::test_support::spawn_http(app).await;
		let client = LlmClient::new_embed_only(&url, "m");
		let (vecs, fails) = embed_chunks(&client, &["a".into(), "b".into()]).await;
		assert_eq!(vecs.len(), 2);
		assert!(
			vecs.iter().all(|v| !v.is_empty()),
			"per-item fallback embedded each chunk"
		);
		assert!(fails.is_empty());
	}

	#[tokio::test]
	async fn embed_with_retry_treats_an_empty_response_as_permanent() {
		// `{"embeddings": []}` decodes to EmptyEmbedding, which is_transient == false,
		// so embed_with_retry bails immediately as permanent (no retry storm).
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|| async { axum::Json(json!({ "embeddings": [] })) }),
		);
		let (url, _server) = crate::test_support::spawn_http(app).await;
		let client = LlmClient::new_embed_only(&url, "m");
		let fail = embed_with_retry(&client, "x", "chunk", 0)
			.await
			.unwrap_err();
		assert_eq!(fail.class, "permanent");
		assert_eq!(fail.scope, "chunk");
	}

	#[tokio::test]
	async fn embed_with_retry_treats_a_connection_failure_as_transient() {
		// Dead port -> connect error -> transient -> exhausts the retry schedule.
		let client = LlmClient::new_embed_only("http://127.0.0.1:1", "m");
		let fail = embed_with_retry(&client, "x", "document", 3)
			.await
			.unwrap_err();
		assert_eq!(fail.class, "transient");
		assert_eq!(fail.chunk_index, 3);
	}

	#[tokio::test]
	async fn embed_chunks_empty_input_short_circuits_to_empty() {
		let (url, _server) = crate::test_support::spawn_http(echo_count_app()).await;
		let client = LlmClient::new_embed_only(&url, "m");
		let (vecs, fails) = embed_chunks(&client, &[]).await;
		assert!(vecs.is_empty() && fails.is_empty());
	}
}
