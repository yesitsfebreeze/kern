use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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

	/// Stream a chat completion as a sequence of content-delta strings.
	/// `messages` is the full multi-turn context (role/content pairs). Yields each
	/// non-empty token delta in order; ends when the server sends `[DONE]` or the
	/// body closes. Errors (HTTP, network) surface as a single `Err` item.
	pub fn complete_stream(
		&self,
		messages: Vec<(String, String)>,
	) -> impl futures_core::Stream<Item = Result<String, LlmError>> + Send {
		let client = self.clone();
		async_stream::stream! {
			let url = format!("{}/v1/chat/completions", client.inner.reason_url);
			let msgs: Vec<serde_json::Value> = messages
				.iter()
				.map(|(r, c)| serde_json::json!({"role": r, "content": c}))
				.collect();
			let body = serde_json::json!({
				"model": client.inner.reason_model,
				"messages": msgs,
				"stream": true,
			});
			let resp = match client.inner.http.post(&url)
				.headers(client.inner.reason_headers.clone())
				.json(&body)
				.send()
				.await
			{
				Ok(r) => r,
				Err(e) => { yield Err(LlmError::from(e)); return; }
			};
			let status = resp.status().as_u16();
			if status >= 400 {
				let body = resp.text().await.unwrap_or_default();
				yield Err(LlmError::Api { status, body });
				return;
			}
			let mut stream = resp.bytes_stream();
			let mut buf = String::new();
			use futures_util::StreamExt as _;
			while let Some(chunk) = stream.next().await {
				let chunk = match chunk {
					Ok(b) => b,
					Err(e) => { yield Err(LlmError::from(e)); return; }
				};
				buf.push_str(&String::from_utf8_lossy(&chunk));
				while let Some(nl) = buf.find('\n') {
					let line: String = buf.drain(..=nl).collect();
					match parse_sse_line(line.trim_end()) {
						Some(SseDelta::Done) => return,
						Some(SseDelta::Token(t)) if !t.is_empty() => yield Ok(t),
						_ => {}
					}
				}
			}
		}
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

#[derive(Debug, PartialEq)]
enum SseDelta {
	Token(String),
	Done,
}

/// Parse one SSE line from an OpenAI/ollama streaming chat response.
/// `data: [DONE]` → `Done`; `data: {json}` → `Token(delta.content)`; anything
/// else (blank lines, comments, non-`data:` fields) → `None`.
fn parse_sse_line(line: &str) -> Option<SseDelta> {
	let rest = line.strip_prefix("data:")?.trim();
	if rest == "[DONE]" {
		return Some(SseDelta::Done);
	}
	let v: Value = serde_json::from_str(rest).ok()?;
	let content = v
		.get("choices")
		.and_then(|c| c.get(0))
		.and_then(|c| c.get("delta"))
		.and_then(|d| d.get("content"))
		.and_then(Value::as_str)
		.unwrap_or("");
	Some(SseDelta::Token(content.to_string()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sse_line_yields_token_then_done() {
		assert_eq!(
			parse_sse_line(r#"data: {"choices":[{"delta":{"content":"He"}}]}"#),
			Some(SseDelta::Token("He".to_string()))
		);
		assert_eq!(parse_sse_line("data: [DONE]"), Some(SseDelta::Done));
		assert_eq!(parse_sse_line(""), None);
		assert_eq!(parse_sse_line(": keep-alive"), None);
		assert_eq!(
			parse_sse_line(r#"data: {"choices":[{"delta":{}}]}"#),
			Some(SseDelta::Token(String::new()))
		);
	}

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
