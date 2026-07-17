//! Provider-agnostic LLM dispatch: `embed`, `complete`, and `answer` legs, each
//! with a native Ollama path and an OpenAI-compat path ([`wants_native`] picks).

use futures_util::StreamExt as _;
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

/// Retry a failed batch embed as a single only for batch-specific failures
/// (transient / empty batch) — a permanent client error fails identically single.
fn should_retry_single(err: &LlmError) -> bool {
	is_transient(err) || matches!(err, LlmError::EmptyEmbedding)
}

/// Parameters for [`Client::answer`]. `num_predict` caps generated tokens —
/// `None` for a real answer, `Some(1)` for the warm ping.
pub struct AnswerParams {
	pub messages: Vec<(String, String)>,
	pub stream: bool,
	pub num_predict: Option<u64>,
}

/// One LLM endpoint. Empty fields fall back per [`Client::new`] (answer/embed →
/// reason), so `Endpoint::default()` means "reuse the reason endpoint".
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
	answer_url: String,
	answer_model: String,
	answer_headers: HeaderMap,
	answer_native: bool,
	embed_url: String,
	embed_model: String,
	embed_headers: HeaderMap,
	embed_native: bool,
	http: reqwest::Client,
	/// Serving pins the reason model to CPU (`num_gpu:0`) so a distillation
	/// burst can't evict the embedder + answer model; eval flips this.
	reason_gpu: bool,
	/// Sampling seed for reason calls (both paths); eval sets it, serving doesn't.
	seed: Option<i64>,
	/// Reason-call temperature override; eval pins the judge to 0.0.
	temperature: Option<f64>,
}

impl Client {
	/// Empty `answer`/`embed` fields fall back to `reason` (embed: url+key;
	/// answer: url+key+model — an empty answer model would 400 on `/ask`).
	pub fn new(reason: Endpoint, answer: Endpoint, embed: Endpoint) -> Self {
		fn or<'a>(v: &'a str, fallback: &'a str) -> &'a str {
			if v.is_empty() {
				fallback
			} else {
				v
			}
		}
		let embed_url = or(&embed.url, &reason.url);
		let embed_key = or(&embed.key, &reason.key);
		let answer_url = or(&answer.url, &reason.url);
		let answer_key = or(&answer.key, &reason.key);
		let answer_model = or(&answer.model, &reason.model);
		// Flags are decided on the configured URL — normalize strips the `/v1`
		// that marks an OpenAI-compat server.
		let reason_native = wants_native(&reason.url);
		let answer_native = wants_native(answer_url);
		let embed_native = wants_native(embed_url);
		let normalize = |u: &str| {
			let u = u.trim_end_matches('/');
			u.strip_suffix("/v1").unwrap_or(u).to_string()
		};
		let http = reqwest::Client::builder()
			.timeout(Duration::from_secs(120))
			// Short connect bound: a dead endpoint must fail fast (transient -> retry)
			// — WSL passthrough to a closed port hangs rather than refusing.
			.connect_timeout(Duration::from_secs(3))
			.build()
			.expect("failed to build HTTP client");
		Self {
			inner: Arc::new(Inner {
				reason_url: normalize(&reason.url),
				reason_model: reason.model.clone(),
				reason_headers: make_headers(&reason.key),
				reason_native,
				answer_url: normalize(answer_url),
				answer_model: answer_model.to_string(),
				answer_headers: make_headers(answer_key),
				answer_native,
				embed_url: normalize(embed_url),
				embed_model: embed.model.clone(),
				embed_headers: make_headers(embed_key),
				embed_native,
				http,
				reason_gpu: false,
				seed: None,
				temperature: None,
			}),
		}
	}

	/// Eval-mode client: reason calls may use the GPU (they ARE the workload)
	/// and sampling is seeded so multi-seed runs are reproducible.
	pub fn for_eval(mut self, seed: i64) -> Self {
		let inner = Arc::make_mut(&mut self.inner);
		inner.reason_gpu = true;
		inner.seed = Some(seed);
		self
	}

	/// Pin the reason-call sampling temperature (both reason paths).
	pub fn with_temperature(mut self, t: f64) -> Self {
		Arc::make_mut(&mut self.inner).temperature = Some(t);
		self
	}

	/// Whether serving should force reason calls onto the CPU (`num_gpu:0`).
	///
	/// The pin exists for ONE reason: a distillation burst on a reason model that
	/// is a *different, larger* model than the answerer evicts the embedder +
	/// answer model from an 8 GB GPU and thrashes `/ask`. When reason and answer
	/// resolve to the same model on the same endpoint there is nothing to evict —
	/// one Ollama runner serves both legs — so the pin has no justification left
	/// and is actively harmful: Ollama keys a runner by its placement, the first
	/// call wins, and a reason call would strand the shared runner on the CPU
	/// where every subsequent `/ask` then pays CPU inference (measured: same tag,
	/// `num_gpu:0` first, and the later GPU-allowed call silently reuses the CPU
	/// runner rather than starting a second one).
	///
	/// Eval always clears the pin: there reason calls ARE the workload.
	fn pins_reason_to_cpu(&self) -> bool {
		if self.inner.reason_gpu {
			return false;
		}
		let shares_answer_runner = self.inner.reason_url == self.inner.answer_url
			&& self.inner.reason_model == self.inner.answer_model;
		!shares_answer_runner
	}

	pub fn new_embed_only(embed_url: &str, embed_model: &str) -> Self {
		Self::new(
			Endpoint::default(),
			Endpoint::default(),
			Endpoint::new(embed_url, embed_model, ""),
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

	/// `/api/embed` body; `input` is one string or an array. Only the NATIVE
	/// endpoint honors `num_ctx`/`keep_alive`; `truncate` clips instead of erroring.
	fn embed_body(&self, input: Value) -> Value {
		serde_json::json!({
			"model": self.inner.embed_model,
			"input": input,
			"truncate": true,
			"keep_alive": EMBED_KEEP_ALIVE,
			"options": { "num_ctx": EMBED_NUM_CTX },
		})
	}

	/// POST `body` as JSON, mapping any non-2xx to [`LlmError::Api`]. Body
	/// decoding stays with the caller — each path parses a different shape.
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
			let mut options = serde_json::json!({ "num_ctx": REASON_NUM_CTX });
			if self.pins_reason_to_cpu() {
				options["num_gpu"] = 0.into();
			}
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
		let content = c.message.content;
		if content.is_empty() {
			return Err(LlmError::EmptyCompletion);
		}
		Ok(content)
	}

	/// Answer-model entry point (`/ask` UI, `query --answer`, warm ping). Yields
	/// each non-empty content delta in order; errors surface as a single `Err`.
	pub fn answer(
		&self,
		params: AnswerParams,
	) -> impl futures_core::Stream<Item = Result<String, LlmError>> + Send {
		let client = self.clone();
		async_stream::stream! {
			let msgs: Vec<Value> = params
				.messages
				.iter()
				.map(|(r, c)| serde_json::json!({"role": r, "content": c}))
				.collect();

			// explicit fn-ptr unifies both arms; closures have distinct anonymous types
			let (resp, parser): (_, fn(&str) -> Option<ChatLine>) =
				if client.inner.answer_native {
					let url = format!("{}/api/chat", client.inner.answer_url);
					let mut options = serde_json::json!({ "num_ctx": ANSWER_NUM_CTX });
					if let Some(n) = params.num_predict { options["num_predict"] = n.into(); }
					let body = serde_json::json!({
						"model": client.inner.answer_model,
						"messages": msgs,
						"stream": params.stream,
						"think": false,
						"keep_alive": ANSWER_KEEP_ALIVE,
						"options": options,
					});
					let resp = match client.post_checked(&url, &client.inner.answer_headers, &body, LLM_TIMEOUT).await {
						Ok(r) => r,
						Err(e) => { yield Err(e); return; }
					};
					if !params.stream {
						match resp.json::<ChatLine>().await {
							Ok(line) => match line.content.filter(|t| !t.is_empty()) {
								Some(t) => { yield Ok(t); }
								None => { yield Err(LlmError::EmptyCompletion); }
							},
							Err(e) => { yield Err(LlmError::from(e)); }
						}
						return;
					}
					(resp, parse_chat_line)
				} else {
					let url = format!("{}/v1/chat/completions", client.inner.answer_url);
					let mut body = serde_json::json!({
						"model": client.inner.answer_model,
						"messages": msgs,
						"stream": params.stream,
					});
					if let Some(n) = params.num_predict { body["max_tokens"] = n.into(); }
					let resp = match client.post_checked(&url, &client.inner.answer_headers, &body, LLM_TIMEOUT).await {
						Ok(r) => r,
						Err(e) => { yield Err(e); return; }
					};
					if !params.stream {
						match resp.json::<ChatResponse>().await {
							Ok(parsed) => {
								let [c] = parsed.choices;
								let t = c.message.content;
								if t.is_empty() { yield Err(LlmError::EmptyCompletion); return; }
								yield Ok(t);
							}
							Err(e) => { yield Err(LlmError::from(e)); }
						}
						return;
					}
					(resp, parse_sse_delta)
				};
			let mut stream = resp.bytes_stream();
			let mut buf: Vec<u8> = Vec::new();
			let mut tokens: Vec<String> = Vec::new();
			while let Some(chunk) = stream.next().await {
				let chunk = match chunk { Ok(b) => b, Err(e) => { yield Err(LlmError::from(e)); return; } };
				buf.extend_from_slice(&chunk);
				let done = drain_stream_lines(&mut buf, &mut tokens, parser);
				for t in tokens.drain(..) { yield Ok(t); }
				if done { return; }
			}
		}
	}

	pub fn complete_func(&self) -> impl Fn(&str) -> String + Send + Sync + 'static {
		let client = self.clone();
		move |prompt: &str| {
			let client = client.clone();
			let prompt = prompt.to_string();
			// No runtime or a completion error both collapse to "" — the distill /
			// edge-label callers treat that as "no output".
			block_on_in_place(client.complete(&prompt))
				.and_then(Result::ok)
				.unwrap_or_default()
		}
	}
}

/// Sync bridge on a runtime worker thread: `block_in_place` makes `block_on`
/// legal here (plain `block_on` panics). `None` outside any runtime.
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

/// Response from Ollama's native `/api/embed`. `embeddings` preserves the order
/// of the request `input` (one row per input string).
#[derive(Deserialize)]
struct NativeEmbedResponse {
	embeddings: Vec<Vec<f32>>,
}

/// Response from OpenAI-compat `/v1/embeddings`. Order is NOT guaranteed —
/// callers must sort by `index` before returning.
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

/// Ollama's 32k default allocates a KV cache big enough to spill the answer
/// model off the GPU (VRAM numbers in the note); 8192 keeps it GPU-resident.
const ANSWER_NUM_CTX: u64 = 8192;

/// Keep the answer model resident (`/v1` ignores `keep_alive`). Paired with the
/// ~4-min warm ping so a user `/ask` never pays a cold reload.
const ANSWER_KEEP_ALIVE: &str = "10m";

/// Without a cap Ollama allocates the model's DEFAULT-context KV cache, which
/// cannot share an 8 GB GPU with the answer model (VRAM numbers in the note).
const EMBED_NUM_CTX: u64 = 2048;

/// Keep the embedder resident; same rationale as [`ANSWER_KEEP_ALIVE`].
const EMBED_KEEP_ALIVE: &str = "10m";

/// Reason prompts are bounded and a larger window only slows CPU prefill.
const REASON_NUM_CTX: u64 = 8192;

/// Short keep-alive frees the large CPU model's RAM between distillation bursts.
const REASON_KEEP_ALIVE: &str = "2m";

/// Overrides the client's 120 s default — slow CPU inference, large RAG prompts,
/// or long streaming answers can run well past it.
const LLM_TIMEOUT: Duration = Duration::from_secs(600);

/// Pinned so embed timeouts stay stable if the client-level default changes.
const EMBED_TIMEOUT: Duration = Duration::from_secs(120);

fn is_local_ollama(url: &str) -> bool {
	url.contains("//localhost") || url.contains("//127.0.0.1") || url.contains(":11434")
}

/// Local URLs get Ollama's native `/api/*`; an explicit `/v1` suffix opts into
/// OpenAI-compat regardless of host (vLLM, llama-server).
fn wants_native(url: &str) -> bool {
	!url.trim_end_matches('/').ends_with("/v1") && is_local_ollama(url)
}

/// One parsed streaming event from either backend. The terminal chunk may
/// carry both content and `done:true` — emit content before acting on `done`.
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
struct SseChunk {
	choices: [SseChoice; 1],
}

#[derive(Deserialize)]
struct SseChoice {
	delta: ContentDelta,
	finish_reason: Option<String>,
}

fn parse_sse_delta(line: &str) -> Option<ChatLine> {
	let data = line.strip_prefix("data: ")?;
	if data == "[DONE]" {
		return Some(ChatLine {
			content: None,
			done: true,
		});
	}
	let chunk: SseChunk = serde_json::from_str(data).ok()?;
	let [choice] = chunk.choices;
	let done = matches!(choice.finish_reason.as_deref(), Some(r) if !r.is_empty());
	Some(ChatLine {
		content: choice.delta.content,
		done,
	})
}

#[derive(Deserialize)]
struct ContentDelta {
	content: Option<String>,
}

fn parse_chat_line(line: &str) -> Option<ChatLine> {
	if line.is_empty() {
		return None;
	}
	serde_json::from_str(line).ok()
}

fn drain_stream_lines<F>(buf: &mut Vec<u8>, tokens: &mut Vec<String>, parser: F) -> bool
where
	F: Fn(&str) -> Option<ChatLine>,
{
	let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') else {
		return false;
	};
	let mut done = false;
	for line in buf[..=last_nl]
		.split(|&b| b == b'\n')
		.filter(|s| !s.is_empty())
	{
		if let Ok(s) = std::str::from_utf8(line) {
			if let Some(cl) = parser(s.trim_end()) {
				if let Some(t) = cl.content {
					if !t.is_empty() {
						tokens.push(t);
					}
				}
				if cl.done {
					done = true;
					break;
				}
			}
		}
	}
	buf.drain(..=last_nl);
	done
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chat_line_yields_token_then_done() {
		assert_eq!(
			parse_chat_line(r#"{"message":{"role":"assistant","content":"He"},"done":false}"#),
			Some(ChatLine {
				content: Some("He".to_string()),
				done: false
			})
		);
		assert_eq!(
			parse_chat_line(r#"{"message":{"content":""},"done":true,"done_reason":"stop"}"#),
			Some(ChatLine {
				content: Some(String::new()),
				done: true
			})
		);
		// Terminal chunk with both content and done=true — content must survive.
		assert_eq!(
			parse_chat_line(r#"{"message":{"content":"Full answer."},"done":true}"#),
			Some(ChatLine {
				content: Some("Full answer.".to_string()),
				done: true
			})
		);
		assert_eq!(parse_chat_line(""), None);
		assert_eq!(parse_chat_line("not json"), None);
		assert_eq!(
			parse_chat_line(r#"{"message":{},"done":false}"#),
			Some(ChatLine {
				content: None,
				done: false
			})
		);
	}

	#[test]
	fn permanent_client_errors_do_not_retry_single() {
		// 400/401 fail identically as a single call — no wasted second round-trip.
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
		// A remote OpenAI-compat host is NOT local — must stay on /v1.
		assert!(!is_local_ollama("https://api.openai.com"));
		// Host substring must be anchored to the authority, not matched anywhere
		// in the URL — `notlocalhost.com` is NOT localhost.
		assert!(!is_local_ollama("http://notlocalhost.com"));
	}

	#[test]
	fn explicit_v1_suffix_forces_openai_compat_even_on_localhost() {
		assert!(wants_native("http://localhost:11434"));
		assert!(wants_native("http://localhost:11434/"));
		// Local vLLM / llama-server: `/v1` opts out of the native path.
		assert!(!wants_native("http://localhost:8000/v1"));
		assert!(!wants_native("http://127.0.0.1:8000/v1/"));
		assert!(!wants_native("https://api.openai.com/v1"));
	}

	fn client_with(reason: (&str, &str), answer: (&str, &str)) -> Client {
		Client::new(
			Endpoint::new(reason.0, reason.1, ""),
			Endpoint::new(answer.0, answer.1, ""),
			Endpoint::default(),
		)
	}

	/// Distinct reason/answer models compete for one 8 GB GPU, so serving keeps
	/// the CPU pin that stops a distillation burst evicting `/ask`.
	#[test]
	fn distinct_reason_and_answer_models_keep_the_serving_cpu_pin() {
		let c = client_with(
			("http://localhost:11434", "qwen2.5:7b"),
			("http://localhost:11434", "qwen3.5:4b"),
		);
		assert!(c.pins_reason_to_cpu());
	}

	/// One model on one endpoint means one Ollama runner for both legs: nothing
	/// can evict anything, and pinning would strand `/ask` on a CPU runner.
	#[test]
	fn shared_reason_and_answer_model_drops_the_cpu_pin() {
		let c = client_with(
			("http://localhost:11434", "granite4:3b"),
			("http://localhost:11434", "granite4:3b"),
		);
		assert!(!c.pins_reason_to_cpu());

		// The stock config path: an empty answer endpoint falls back to reason,
		// which is exactly how a zero-config install resolves both legs.
		let stock = Client::new(
			Endpoint::new("http://localhost:11434", "granite4:3b", ""),
			Endpoint::default(),
			Endpoint::default(),
		);
		assert!(!stock.pins_reason_to_cpu());
	}

	/// Same model tag on DIFFERENT hosts is two runners on two machines — the
	/// shared-runner reasoning does not apply, so the pin stays.
	#[test]
	fn same_model_on_different_endpoints_keeps_the_cpu_pin() {
		let c = client_with(
			("http://localhost:11434", "granite4:3b"),
			("http://gpu-box:11434", "granite4:3b"),
		);
		assert!(c.pins_reason_to_cpu());
	}

	/// Eval clears the pin regardless: there reason calls ARE the workload.
	#[test]
	fn eval_never_pins_reason_to_cpu() {
		let c = client_with(
			("http://localhost:11434", "qwen2.5:7b"),
			("http://localhost:11434", "qwen3.5:4b"),
		)
		.for_eval(7);
		assert!(!c.pins_reason_to_cpu());
	}

	/// The stub distinguishes batch (array `input`) from the single retry (string):
	/// the fallback fires only after a retry-worthy batch failure.
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
		let client = Client::new_embed_only(&url, "m");
		let v = client
			.embed("hello")
			.await
			.expect("transient batch -> single retry succeeds");
		assert_eq!(v, vec![1.0, 2.0, 3.0]);
	}

	/// An empty batch response is retry-worthy too — the single retry recovers.
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
		let client = Client::new_embed_only(&url, "m");
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
		let client = Client::new_embed_only(&url, "m");
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
