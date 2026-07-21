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

fn should_retry_single(err: &LlmError) -> bool {
	is_transient(err) || matches!(err, LlmError::EmptyEmbedding)
}

#[derive(Default, Clone)]
pub struct Endpoint {
	pub url: String,
	pub model: String,
	pub key: String,
}

impl Endpoint {
	pub fn new(url: &str, model: &str, key: &str) -> Self {
		Self {
			url: url.to_string(),
			model: model.to_string(),
			key: key.to_string(),
		}
	}
}

#[derive(Clone)]
pub struct Client {
	inner: Arc<Inner>,
}

#[derive(Clone)]
struct Inner {
	reason_url: String,
	reason_model: String,
	reason_headers: HeaderMap,
	reason_native: bool,
	embed_url: String,
	embed_model: String,
	embed_headers: HeaderMap,
	embed_native: bool,
	http: reqwest::Client,
	seed: Option<i64>,
	temperature: Option<f64>,
	num_ctx: Option<u64>,
}

impl Client {
	pub fn new(reason: Endpoint, embed: Endpoint) -> Self {
		fn or<'a>(v: &'a str, fallback: &'a str) -> &'a str {
			if v.is_empty() {
				fallback
			} else {
				v
			}
		}
		let embed_url = or(&embed.url, &reason.url);
		let embed_key = or(&embed.key, &reason.key);
		// wants_native reads the pre-normalize URL: normalize strips the `/v1` that marks OpenAI-compat.
		let reason_native = wants_native(&reason.url);
		let embed_native = wants_native(embed_url);
		let normalize = |u: &str| {
			let u = u.trim_end_matches('/');
			u.strip_suffix("/v1").unwrap_or(u).to_string()
		};
		let http = reqwest::Client::builder()
			.timeout(Duration::from_secs(120))
			// Short connect bound so a dead endpoint fails fast (transient -> retry):
			// WSL passthrough to a closed port hangs rather than refusing.
			.connect_timeout(Duration::from_secs(3))
			.build()
			.expect("failed to build HTTP client");
		Self {
			inner: Arc::new(Inner {
				reason_url: normalize(&reason.url),
				reason_model: reason.model.clone(),
				reason_headers: make_headers(&reason.key),
				reason_native,
				embed_url: normalize(embed_url),
				embed_model: embed.model.clone(),
				embed_headers: make_headers(embed_key),
				embed_native,
				http,
				seed: None,
				temperature: None,
				num_ctx: None,
			}),
		}
	}

	pub fn for_eval(mut self, seed: i64) -> Self {
		Arc::make_mut(&mut self.inner).seed = Some(seed);
		self
	}

	pub fn with_temperature(mut self, t: f64) -> Self {
		Arc::make_mut(&mut self.inner).temperature = Some(t);
		self
	}

	// Ollama-native `complete` only; the OpenAI-compat path has no client-side window.
	pub fn with_num_ctx(mut self, n: u64) -> Self {
		Arc::make_mut(&mut self.inner).num_ctx = Some(n);
		self
	}

	pub fn has_reason(&self) -> bool {
		!self.inner.reason_url.is_empty()
	}

	pub fn new_embed_only(embed_url: &str, embed_model: &str, embed_key: &str) -> Self {
		Self::new(
			Endpoint::default(),
			Endpoint::new(embed_url, embed_model, embed_key),
		)
	}

	pub async fn embed(&self, text: &str) -> Result<Vec<f32>, LlmError> {
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

	fn embed_body(&self, input: Value) -> Value {
		serde_json::json!({
			"model": self.inner.embed_model,
			"input": input,
			"truncate": true,
			"keep_alive": EMBED_KEEP_ALIVE,
			"options": { "num_ctx": EMBED_NUM_CTX },
		})
	}

	async fn post_checked<T: Serialize + ?Sized>(
		&self,
		url: &str,
		headers: &HeaderMap,
		body: &T,
		timeout: Duration,
	) -> Result<reqwest::Response, LlmError> {
		check_status(
			self
				.inner
				.http
				.post(url)
				.headers(headers.clone())
				.json(body)
				.timeout(timeout)
				.send()
				.await?,
		)
		.await
	}

	pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, LlmError> {
		if self.inner.embed_native {
			let url = format!("{}/api/embed", self.inner.embed_url);
			let body = self.embed_body(serde_json::json!(texts));
			let resp = self
				.post_checked(&url, &self.inner.embed_headers, &body, EMBED_TIMEOUT)
				.await?;
			// /api/embed preserves input order in `embeddings`, so no index sort needed.
			let parsed: NativeEmbedResponse = resp.json().await?;
			if parsed.embeddings.is_empty() {
				return Err(LlmError::EmptyEmbedding);
			}
			return Ok(parsed.embeddings);
		}
		let url = format!("{}/v1/embeddings", self.inner.embed_url);
		let body = serde_json::json!({ "model": self.inner.embed_model, "input": texts });
		let resp = self
			.post_checked(&url, &self.inner.embed_headers, &body, EMBED_TIMEOUT)
			.await?;
		let mut parsed: OpenAiEmbedResponse = resp.json().await?;
		if parsed.data.is_empty() {
			return Err(LlmError::EmptyEmbedding);
		}
		// OpenAI does not guarantee order — sort by index before returning.
		parsed.data.sort_by_key(|i| i.index);
		Ok(parsed.data.into_iter().map(|i| i.embedding).collect())
	}

	async fn embed_single(&self, text: &str) -> Result<Vec<f32>, LlmError> {
		if self.inner.embed_native {
			let url = format!("{}/api/embed", self.inner.embed_url);
			let body = self.embed_body(serde_json::json!(text));
			let resp = self
				.post_checked(&url, &self.inner.embed_headers, &body, EMBED_TIMEOUT)
				.await?;
			let parsed: NativeEmbedResponse = resp.json().await?;
			return parsed
				.embeddings
				.into_iter()
				.next()
				.ok_or(LlmError::EmptyEmbedding);
		}
		let url = format!("{}/v1/embeddings", self.inner.embed_url);
		let body = serde_json::json!({ "model": self.inner.embed_model, "input": text });
		let resp = self
			.post_checked(&url, &self.inner.embed_headers, &body, EMBED_TIMEOUT)
			.await?;
		let parsed: OpenAiEmbedResponse = resp.json().await?;
		parsed
			.data
			.into_iter()
			.next()
			.map(|i| i.embedding)
			.ok_or(LlmError::EmptyEmbedding)
	}

	pub async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
		if self.inner.reason_native {
			let url = format!("{}/api/chat", self.inner.reason_url);
			let mut options =
				serde_json::json!({ "num_ctx": self.inner.num_ctx.unwrap_or(REASON_NUM_CTX) });
			if let Some(s) = self.inner.seed {
				options["seed"] = s.into();
			}
			if let Some(t) = self.inner.temperature {
				options["temperature"] = t.into();
			}
			let body = serde_json::json!({
				"model": self.inner.reason_model,
				"messages": [{"role": "user", "content": prompt}],
				"stream": false,
				"think": false,
				"keep_alive": REASON_KEEP_ALIVE,
				"options": options,
			});
			let resp = self
				.post_checked(&url, &self.inner.reason_headers, &body, LLM_TIMEOUT)
				.await?;
			return resp
				.json::<ChatLine>()
				.await?
				.content
				.map(|t| strip_think(&t))
				.filter(|t| !t.is_empty())
				.ok_or(LlmError::EmptyCompletion);
		}

		let url = format!("{}/v1/chat/completions", self.inner.reason_url);
		let body = ChatRequest {
			model: &self.inner.reason_model,
			messages: vec![ChatMessage {
				role: "user",
				content: prompt,
			}],
			seed: self.inner.seed,
			temperature: self.inner.temperature,
		};
		let resp = self
			.post_checked(&url, &self.inner.reason_headers, &body, LLM_TIMEOUT)
			.await?;
		let parsed: ChatResponse = resp.json().await?;
		let [c] = parsed.choices;
		let content = strip_think(&c.message.content);
		if content.is_empty() {
			return Err(LlmError::EmptyCompletion);
		}
		Ok(content)
	}

	pub fn complete_func(&self) -> impl Fn(&str) -> String + Send + Sync + 'static {
		let client = self.clone();
		move |prompt: &str| {
			let client = client.clone();
			let prompt = prompt.to_string();
			block_on_in_place(client.complete(&prompt))
				.and_then(Result::ok)
				.unwrap_or_default()
		}
	}
}

// `block_in_place` makes `block_on` legal on a runtime worker (plain `block_on` panics); `None` outside any runtime.
pub(crate) fn block_on_in_place<F: std::future::Future>(fut: F) -> Option<F::Output> {
	let handle = tokio::runtime::Handle::try_current().ok()?;
	Some(tokio::task::block_in_place(|| handle.block_on(fut)))
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, LlmError> {
	let status = resp.status().as_u16();
	if status >= 400 {
		let body = resp.text().await.unwrap_or_default();
		return Err(LlmError::Api { status, body });
	}
	Ok(resp)
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

#[derive(Deserialize)]
struct NativeEmbedResponse {
	embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
	data: Vec<OpenAiEmbedItem>,
}

#[derive(Deserialize)]
struct OpenAiEmbedItem {
	embedding: Vec<f32>,
	index: usize,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
	model: &'a str,
	messages: Vec<ChatMessage<'a>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	seed: Option<i64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	temperature: Option<f64>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
	role: &'a str,
	content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
	choices: [ChatChoice; 1],
}

#[derive(Deserialize)]
struct ChatChoice {
	message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
	content: String,
}

// Without a cap Ollama's default-context KV cache can't share a small GPU with the reason model.
const EMBED_NUM_CTX: u64 = 2048;

// Keep the embedder resident (`/v1` ignores `keep_alive`); avoids cold reloads between calls.
const EMBED_KEEP_ALIVE: &str = "10m";

const REASON_NUM_CTX: u64 = 8192;

const REASON_KEEP_ALIVE: &str = "2m";

// Overrides the client's 120 s default: slow CPU inference / large RAG prompts / long streams run past it.
const LLM_TIMEOUT: Duration = Duration::from_secs(600);

const EMBED_TIMEOUT: Duration = Duration::from_secs(120);

fn is_local_ollama(url: &str) -> bool {
	url.contains("//localhost") || url.contains("//127.0.0.1") || url.contains(":11434")
}

fn wants_native(url: &str) -> bool {
	!url.trim_end_matches('/').ends_with("/v1") && is_local_ollama(url)
}

// Terminal chunk may carry both content and `done:true` — emit content before acting on `done`.
#[derive(Debug, PartialEq)]
struct ChatLine {
	content: Option<String>,
	done: bool,
}

impl<'de> Deserialize<'de> for ChatLine {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		struct Wire {
			message: Option<ContentDelta>,
			#[serde(default)]
			done: bool,
		}
		let w = Wire::deserialize(d)?;
		Ok(ChatLine {
			content: w.message.and_then(|m| m.content),
			done: w.done,
		})
	}
}

#[derive(Deserialize)]
struct ContentDelta {
	content: Option<String>,
}

// Reasoning models can leak chain-of-thought into content even with think:false;
// the answer is whatever follows the last closing tag.
fn strip_think(s: &str) -> String {
	let after = match s.rfind("</think>") {
		Some(i) => &s[i + "</think>".len()..],
		None => s,
	};
	let clean = match after.find("<think>") {
		Some(i) => &after[..i],
		None => after,
	};
	clean.trim().to_string()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn strip_think_recovers_answer_after_leaked_reasoning() {
		assert_eq!(strip_think("plain answer"), "plain answer");
		assert_eq!(strip_think("<think>hmm</think>yes"), "yes");
		assert_eq!(strip_think("leaked reasoning</think>yes"), "yes");
		assert_eq!(strip_think("a</think>b</think>final"), "final");
		assert_eq!(strip_think("answer<think>unclosed trailing"), "answer");
		assert_eq!(strip_think("<think>only reasoning"), "");
	}

	#[test]
	fn permanent_client_errors_do_not_retry_single() {
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

	#[test]
	fn local_ollama_markers_match_loopback_and_default_port() {
		assert!(is_local_ollama("http://localhost"));
		assert!(is_local_ollama("http://127.0.0.1:9999"));
		assert!(is_local_ollama("http://ollama:11434"));
		assert!(!is_local_ollama("https://api.openai.com"));
		assert!(!is_local_ollama("http://notlocalhost.com"));
	}

	#[test]
	fn explicit_v1_suffix_forces_openai_compat_even_on_localhost() {
		assert!(wants_native("http://localhost:11434"));
		assert!(wants_native("http://localhost:11434/"));
		assert!(!wants_native("http://localhost:8000/v1"));
		assert!(!wants_native("http://127.0.0.1:8000/v1/"));
		assert!(!wants_native("https://api.openai.com/v1"));
	}

	#[tokio::test]
	async fn embed_falls_back_to_single_on_transient_batch_error() {
		use axum::http::StatusCode;
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|body: axum::Json<Value>| async move {
				if body.0["input"].is_array() {
					(
						StatusCode::SERVICE_UNAVAILABLE,
						axum::Json(serde_json::json!({ "error": "busy" })),
					)
				} else {
					(
						StatusCode::OK,
						axum::Json(serde_json::json!({ "embeddings": [[1.0, 2.0, 3.0]] })),
					)
				}
			}),
		);
		let (url, _server) = crate::test_support::spawn_http(app).await;
		let client = Client::new_embed_only(&url, "m", "");
		let v = client
			.embed("hello")
			.await
			.expect("transient batch -> single retry succeeds");
		assert_eq!(v, vec![1.0, 2.0, 3.0]);
	}

	#[tokio::test]
	async fn embed_falls_back_to_single_on_empty_batch_response() {
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|body: axum::Json<Value>| async move {
				if body.0["input"].is_array() {
					axum::Json(serde_json::json!({ "embeddings": [] }))
				} else {
					axum::Json(serde_json::json!({ "embeddings": [[9.0]] }))
				}
			}),
		);
		let (url, _server) = crate::test_support::spawn_http(app).await;
		let client = Client::new_embed_only(&url, "m", "");
		let v = client
			.embed("x")
			.await
			.expect("empty batch -> single retry succeeds");
		assert_eq!(v, vec![9.0]);
	}

	#[tokio::test]
	async fn embed_propagates_permanent_batch_error_without_retry() {
		use axum::http::StatusCode;
		use std::sync::atomic::{AtomicUsize, Ordering};
		let hits = Arc::new(AtomicUsize::new(0));
		let h = hits.clone();
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(move |_body: axum::Json<Value>| {
				let h = h.clone();
				async move {
					h.fetch_add(1, Ordering::SeqCst);
					(
						StatusCode::BAD_REQUEST,
						axum::Json(serde_json::json!({ "error": "bad model" })),
					)
				}
			}),
		);
		let (url, _server) = crate::test_support::spawn_http(app).await;
		let client = Client::new_embed_only(&url, "m", "");
		let err = client.embed("hello").await.unwrap_err();
		assert!(
			matches!(err, LlmError::Api { status: 400, .. }),
			"permanent error propagates, got {err:?}"
		);
		assert_eq!(
			hits.load(Ordering::SeqCst),
			1,
			"no wasted single retry on a permanent error"
		);
	}
}
