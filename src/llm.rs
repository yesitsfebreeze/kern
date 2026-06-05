use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
	#[error("HTTP error: {0}")]
	Http(#[from] reqwest::Error),
	#[error("API error ({status}): {body}")]
	Api { status: u16, body: String },
	#[error("empty embedding response")]
	EmptyEmbedding,
	#[error("empty completion response")]
	EmptyCompletion,
}

pub fn is_transient(err: &LlmError) -> bool {
	match err {
		LlmError::Http(e) => {
			if e.is_timeout() || e.is_connect() {
				return true;
			}
			if let Some(s) = e.status() {
				return s.as_u16() >= 500 || s.as_u16() == 429;
			}
			true
		}
		LlmError::Api { status, .. } => *status >= 500 || *status == 429,
		_ => false,
	}
}

/// Whether a failed batch embed warrants a second attempt as a single embed.
/// Only batch-specific failures qualify — transient network/5xx/429 errors, or
/// an empty batch response. A permanent client error (e.g. 400 bad model, 401
/// auth) fails identically as a single call, so it is propagated instead, which
/// lets [`embed_with_retry`](crate::ingest) short-circuit rather than pay a
/// second HTTP round-trip per chunk.
fn should_retry_single(err: &LlmError) -> bool {
	is_transient(err) || matches!(err, LlmError::EmptyEmbedding)
}

#[derive(Clone)]
pub struct Client {
	inner: Arc<Inner>,
}

struct Inner {
	reason_url: String,
	reason_model: String,
	reason_headers: HeaderMap,
	embed_url: String,
	embed_model: String,
	embed_headers: HeaderMap,
	http: reqwest::Client,
}

impl Client {
	pub fn new(
		reason_url: &str,
		reason_model: &str,
		reason_key: &str,
		embed_url: &str,
		embed_model: &str,
		embed_key: &str,
	) -> Self {
		let embed_url = if embed_url.is_empty() {
			reason_url
		} else {
			embed_url
		};
		let embed_key = if embed_key.is_empty() {
			reason_key
		} else {
			embed_key
		};
		let normalize = |u: &str| {
			let u = u.trim_end_matches('/');
			u.strip_suffix("/v1").unwrap_or(u).to_string()
		};
		let http = reqwest::Client::builder()
			.timeout(Duration::from_secs(120))
			.build()
			.expect("failed to build HTTP client");
		Self {
			inner: Arc::new(Inner {
				reason_url: normalize(reason_url),
				reason_model: reason_model.to_string(),
				reason_headers: make_headers(reason_key),
				embed_url: normalize(embed_url),
				embed_model: embed_model.to_string(),
				embed_headers: make_headers(embed_key),
				http,
			}),
		}
	}

	pub fn new_embed_only(embed_url: &str, embed_model: &str) -> Self {
		Self::new("", "", "", embed_url, embed_model, "")
	}

	pub async fn embed(&self, text: &str) -> Result<Vec<f64>, LlmError> {
		match self.embed_batch(&[text.to_string()]).await {
			Ok(mut vecs) => {
				if vecs.is_empty() {
					return Err(LlmError::EmptyEmbedding);
				}
				Ok(vecs.swap_remove(0))
			}
			Err(e) if should_retry_single(&e) => self.embed_single(text).await,
			Err(e) => Err(e),
		}
	}

	pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f64>>, LlmError> {
		let url = format!("{}/v1/embeddings", self.inner.embed_url);
		let body = EmbedBatchRequest {
			model: &self.inner.embed_model,
			input: texts,
		};
		let resp = self
			.inner
			.http
			.post(&url)
			.headers(self.inner.embed_headers.clone())
			.json(&body)
			.send()
			.await?;
		let status = resp.status().as_u16();
		if status >= 400 {
			let body = resp.text().await.unwrap_or_default();
			return Err(LlmError::Api { status, body });
		}
		let mut parsed: EmbedResponse = resp.json().await?;
		if parsed.data.is_empty() {
			return Err(LlmError::EmptyEmbedding);
		}
		parsed.data.sort_by_key(|d| d.index);
		Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
	}

	async fn embed_single(&self, text: &str) -> Result<Vec<f64>, LlmError> {
		let url = format!("{}/v1/embeddings", self.inner.embed_url);
		let body = EmbedSingleRequest {
			model: &self.inner.embed_model,
			input: text,
		};
		let resp = self
			.inner
			.http
			.post(&url)
			.headers(self.inner.embed_headers.clone())
			.json(&body)
			.send()
			.await?;
		let status = resp.status().as_u16();
		if status >= 400 {
			let body = resp.text().await.unwrap_or_default();
			return Err(LlmError::Api { status, body });
		}
		let parsed: EmbedResponse = resp.json().await?;
		parsed
			.data
			.into_iter()
			.next()
			.map(|d| d.embedding)
			.ok_or(LlmError::EmptyEmbedding)
	}

	pub async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
		let url = format!("{}/v1/chat/completions", self.inner.reason_url);
		let body = ChatRequest {
			model: &self.inner.reason_model,
			messages: vec![ChatMessage {
				role: "user",
				content: prompt,
			}],
		};
		let resp = self
			.inner
			.http
			.post(&url)
			.headers(self.inner.reason_headers.clone())
			.json(&body)
			.send()
			.await?;
		let status = resp.status().as_u16();
		if status >= 400 {
			let body = resp.text().await.unwrap_or_default();
			return Err(LlmError::Api { status, body });
		}
		let parsed: ChatResponse = resp.json().await?;
		parsed
			.choices
			.into_iter()
			.next()
			.map(|c| c.message.content)
			.ok_or(LlmError::EmptyCompletion)
	}

	pub fn complete_func(&self) -> impl Fn(&str) -> String + Send + Sync + 'static {
		let client = self.clone();
		move |prompt: &str| {
			let client = client.clone();
			let prompt = prompt.to_string();
			match tokio::runtime::Handle::try_current() {
				Ok(handle) => {
					let result = tokio::task::block_in_place(|| handle.block_on(client.complete(&prompt)));
					result.unwrap_or_default()
				}
				Err(_) => String::new(),
			}
		}
	}
}

fn make_headers(key: &str) -> HeaderMap {
	let mut h = HeaderMap::new();
	h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
	if !key.is_empty() {
		if let Ok(v) = HeaderValue::from_str(&format!("Bearer {key}")) {
			h.insert(AUTHORIZATION, v);
		}
	}
	h
}

#[derive(Serialize)]
struct EmbedBatchRequest<'a> {
	model: &'a str,
	input: &'a [String],
}

#[derive(Serialize)]
struct EmbedSingleRequest<'a> {
	model: &'a str,
	input: &'a str,
}

#[derive(Deserialize)]
struct EmbedResponse {
	data: Vec<EmbedDatum>,
}

#[derive(Deserialize)]
struct EmbedDatum {
	embedding: Vec<f64>,
	#[serde(default)]
	index: usize,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
	model: &'a str,
	messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
	role: &'a str,
	content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
	choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
	message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
	content: String,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn permanent_client_errors_do_not_retry_single() {
		// 400 bad model, 401 auth: a single embed fails identically, so the
		// batch error must propagate (no wasted second round-trip).
		assert!(!should_retry_single(&LlmError::Api {
			status: 400,
			body: String::new()
		}));
		assert!(!should_retry_single(&LlmError::Api {
			status: 401,
			body: String::new()
		}));
		assert!(!should_retry_single(&LlmError::EmptyCompletion));
	}

	#[test]
	fn transient_and_empty_batch_retry_single() {
		assert!(should_retry_single(&LlmError::Api {
			status: 429,
			body: String::new()
		}));
		assert!(should_retry_single(&LlmError::Api {
			status: 503,
			body: String::new()
		}));
		assert!(should_retry_single(&LlmError::EmptyEmbedding));
	}
}
