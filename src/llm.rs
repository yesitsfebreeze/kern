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

/// Whether a failed batch embed warrants a second attempt as a single embed.
/// Only batch-specific failures qualify — transient network/5xx/429 errors, or
/// an empty batch response. A permanent client error (e.g. 400 bad model, 401
/// auth) fails identically as a single call, so it is propagated instead, which
/// lets [`embed_with_retry`](crate::ingest) short-circuit rather than pay a
/// second HTTP round-trip per chunk.
fn should_retry_single(err: &LlmError) -> bool {
	is_transient(err) || matches!(err, LlmError::EmptyEmbedding)
}

/// Parameters for [`Client::answer`]. `messages` is the full role/content turn
/// list (the `/ask` UI passes multi-turn context; single-shot callers pass one
/// `("user", prompt)` pair). `stream` toggles Ollama wire streaming. `num_predict`
/// caps generated tokens — `None` for a real answer, `Some(1)` for the warm ping.
pub struct AnswerParams {
	pub messages: Vec<(String, String)>,
	pub stream: bool,
	pub num_predict: Option<u64>,
}

/// One LLM endpoint: base URL, model tag, optional bearer key. Empty fields fall
/// back per [`Client::new`] (answer/embed → reason), so `Endpoint::default()`
/// means "reuse the reason endpoint".
#[derive(Default, Clone)]
pub struct Endpoint {
	pub url: String,
	pub model: String,
	pub key: String,
}

impl Endpoint {
	pub fn new(url: &str, model: &str, key: &str) -> Self {
		Self { url: url.to_string(), model: model.to_string(), key: key.to_string() }
	}
}

#[derive(Clone)]
pub struct Client {
	inner: Arc<Inner>,
}

struct Inner {
	reason_url: String,
	reason_model: String,
	reason_headers: HeaderMap,
	answer_url: String,
	answer_model: String,
	answer_headers: HeaderMap,
	embed_url: String,
	embed_model: String,
	embed_headers: HeaderMap,
	http: reqwest::Client,
}

impl Client {
	/// Build a client from three endpoints. Empty `answer`/`embed` fields fall back
	/// to `reason`: embed reuses reason's url+key (single-Ollama host), and answer
	/// reuses reason's url+key+model — the common case where only the answer model
	/// differs. An empty answer model would 400 on `/ask`, so it falls back too.
	pub fn new(reason: Endpoint, answer: Endpoint, embed: Endpoint) -> Self {
		fn or<'a>(v: &'a str, fallback: &'a str) -> &'a str {
			if v.is_empty() { fallback } else { v }
		}
		let embed_url = or(&embed.url, &reason.url);
		let embed_key = or(&embed.key, &reason.key);
		let answer_url = or(&answer.url, &reason.url);
		let answer_key = or(&answer.key, &reason.key);
		let answer_model = or(&answer.model, &reason.model);
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
				reason_url: normalize(&reason.url),
				reason_model: reason.model.clone(),
				reason_headers: make_headers(&reason.key),
				answer_url: normalize(answer_url),
				answer_model: answer_model.to_string(),
				answer_headers: make_headers(answer_key),
				embed_url: normalize(embed_url),
				embed_model: embed.model.clone(),
				embed_headers: make_headers(embed_key),
				http,
			}),
		}
	}

	pub fn new_embed_only(embed_url: &str, embed_model: &str) -> Self {
		Self::new(Endpoint::default(), Endpoint::default(), Endpoint::new(embed_url, embed_model, ""))
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

	/// The shared `/api/embed` request body. `input` is either one string or an
	/// array of strings — both native-endpoint shapes.
	///
	/// Ollama's NATIVE /api/embed, not the OpenAI-compat /v1/embeddings: only the
	/// native endpoint honors `options.num_ctx` and `keep_alive`. Without a num_ctx
	/// cap Ollama allocates a KV cache for the model's DEFAULT context (32k for
	/// qwen3-embedding) — that balloons a 0.6b embedder to ~5.8 GB of VRAM, which on
	/// an 8 GB GPU cannot coexist with the answer model, so every `/ask` (embed →
	/// answer) thrashes Ollama swapping the two and, under the multi-daemon forest's
	/// concurrent load, wedges it outright. Capping to EMBED_NUM_CTX holds the
	/// embedder at ~1.5 GB so it stays resident beside the answer model. `truncate`
	/// lets an over-long input clip instead of erroring. See [`Client::answer`] for
	/// the mirror of this on the answer path.
	fn embed_body(&self, input: Value) -> Value {
		serde_json::json!({
			"model": self.inner.embed_model,
			"input": input,
			"truncate": true,
			"keep_alive": EMBED_KEEP_ALIVE,
			"options": { "num_ctx": EMBED_NUM_CTX },
		})
	}

	/// Shared request dispatch: POST `body` as JSON to `url` with `headers`,
	/// applying an optional per-request timeout override, and map any non-2xx to
	/// [`LlmError::Api`]. The single point every embed/reason request flows
	/// through — the four call sites (`embed_batch`, `embed_single`, and both
	/// branches of `complete`) previously inlined this identical block. Decoding
	/// the success body stays with the caller because the paths parse different
	/// shapes (`NativeEmbedResponse`, raw text, `ChatResponse`).
	async fn post_checked<T: Serialize + ?Sized>(
		&self,
		url: &str,
		headers: &HeaderMap,
		body: &T,
		timeout: Option<Duration>,
	) -> Result<reqwest::Response, LlmError> {
		let mut req = self.inner.http.post(url).headers(headers.clone()).json(body);
		if let Some(t) = timeout {
			req = req.timeout(t);
		}
		check_status(req.send().await?).await
	}

	pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f64>>, LlmError> {
		let url = format!("{}/api/embed", self.inner.embed_url);
		let body = self.embed_body(serde_json::json!(texts));
		let resp = self.post_checked(&url, &self.inner.embed_headers, &body, None).await?;
		// /api/embed preserves input order in `embeddings`, so no index sort needed.
		let parsed: NativeEmbedResponse = resp.json().await?;
		if parsed.embeddings.is_empty() {
			return Err(LlmError::EmptyEmbedding);
		}
		Ok(parsed.embeddings)
	}

	async fn embed_single(&self, text: &str) -> Result<Vec<f64>, LlmError> {
		let url = format!("{}/api/embed", self.inner.embed_url);
		let body = self.embed_body(serde_json::json!(text));
		let resp = self.post_checked(&url, &self.inner.embed_headers, &body, None).await?;
		let parsed: NativeEmbedResponse = resp.json().await?;
		parsed
			.embeddings
			.into_iter()
			.next()
			.ok_or(LlmError::EmptyEmbedding)
	}

	pub async fn complete(&self, prompt: &str) -> Result<String, LlmError> {
		// Local Ollama reason: run distillation/edge-proposal on CPU (num_gpu:0)
		// via the native endpoint. Reason is latency-insensitive background work,
		// but the reason model (e.g. qwen2.5:7b, 4.7 GB) cannot share an 8 GB GPU
		// with the embedder + answer model — loading it for a distillation burst
		// evicts both and thrashes the user-facing `/ask`. Forcing it to CPU keeps
		// the GPU entirely for embed+answer so `/ask` stays fast, at the cost of
		// slower (but invisible) distillation. Cloud reason endpoints don't support
		// `num_gpu`, so they keep the OpenAI-compat `/v1` path below.
		if is_local_ollama(&self.inner.reason_url) {
			let url = format!("{}/api/chat", self.inner.reason_url);
			let body = serde_json::json!({
				"model": self.inner.reason_model,
				"messages": [{"role": "user", "content": prompt}],
				"stream": false,
				"think": false,
				"keep_alive": REASON_KEEP_ALIVE,
				"options": { "num_ctx": REASON_NUM_CTX, "num_gpu": 0 },
			});
			let resp = self
				.post_checked(
					&url,
					&self.inner.reason_headers,
					&body,
					Some(Duration::from_secs(600)),
				)
				.await?;
			let text = resp.text().await?;
			// One `stream:false` object; `parse_chat_line` already knows this shape.
			return parse_chat_line(text.trim_end())
				.and_then(|cl| cl.content)
				.ok_or(LlmError::EmptyCompletion);
		}

		let url = format!("{}/v1/chat/completions", self.inner.reason_url);
		let body = ChatRequest {
			model: &self.inner.reason_model,
			messages: vec![ChatMessage {
				role: "user",
				content: prompt,
			}],
		};
		let resp = self.post_checked(&url, &self.inner.reason_headers, &body, None).await?;
		let parsed: ChatResponse = resp.json().await?;
		parsed
			.choices
			.into_iter()
			.next()
			.map(|c| c.message.content)
			.ok_or(LlmError::EmptyCompletion)
	}

	/// The single answer-model entry point. Streams a completion over Ollama's
	/// NATIVE `/api/chat` — used by the `/ask` UI (consumed incrementally), the CLI
	/// `query --answer` (collected into a string), and the keep-alive warm ping
	/// (drained, `num_predict: 1`). All three share one request shape and one
	/// response parser; there is no second answer path.
	///
	/// `/api/chat` (not the OpenAI-compat `/v1`) is the only endpoint that honors
	/// `options.num_ctx` and `keep_alive`, both of which the answer path needs to
	/// stay GPU-resident: `/v1` would cold-load every call and let Ollama's default
	/// 32k KV cache spill a 4b model onto CPU (~2x slower, and it evicts the
	/// embedder). `think:false` skips qwen3.5's hidden reasoning phase — pure
	/// latency for a path that only glues retrieved nodes into prose (ignored by
	/// non-reasoning models).
	///
	/// Yields each non-empty content delta in order; ends when the server reports
	/// `done`. Errors (HTTP, network, 4xx/5xx) surface as a single `Err` item.
	/// `params.stream` toggles wire streaming — `true` delivers tokens as they
	/// arrive, `false` returns one object — but either way the content is yielded
	/// through this same stream, so callers handle both identically.
	pub fn answer(
		&self,
		params: AnswerParams,
	) -> impl futures_core::Stream<Item = Result<String, LlmError>> + Send {
		let client = self.clone();
		async_stream::stream! {
			let url = format!("{}/api/chat", client.inner.answer_url);
			let msgs: Vec<Value> = params
				.messages
				.iter()
				.map(|(r, c)| serde_json::json!({"role": r, "content": c}))
				.collect();
			let mut options = serde_json::json!({ "num_ctx": ANSWER_NUM_CTX });
			if let Some(n) = params.num_predict {
				options["num_predict"] = n.into();
			}
			let body = serde_json::json!({
				"model": client.inner.answer_model,
				"messages": msgs,
				"stream": params.stream,
				"think": false,
				"keep_alive": ANSWER_KEEP_ALIVE,
				"options": options,
			});
			// Override the client's 120s TOTAL timeout: a generation can take far
			// longer than 120s (big RAG prompt + CPU inference), and a total-response
			// timeout would abort it mid-stream as "error decoding response body".
			// 600s is a generous ceiling; tokens still stream as they arrive.
			let resp = match client.inner.http.post(&url)
				.headers(client.inner.answer_headers.clone())
				.timeout(Duration::from_secs(600))
				.json(&body)
				.send()
				.await
			{
				Ok(r) => r,
				Err(e) => { yield Err(LlmError::from(e)); return; }
			};
			let resp = match check_status(resp).await {
				Ok(r) => r,
				Err(e) => { yield Err(e); return; }
			};
			let mut stream = resp.bytes_stream();
			let mut buf: Vec<u8> = Vec::new();
			while let Some(chunk) = stream.next().await {
				let chunk = match chunk {
					Ok(b) => b,
					Err(e) => { yield Err(LlmError::from(e)); return; }
				};
				buf.extend_from_slice(&chunk);
				// Decode only COMPLETE lines, so a multibyte char split across chunks
				// is never lossily decoded mid-sequence. Each full line is valid UTF-8.
				while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
					let raw: Vec<u8> = buf.drain(..=pos).collect();
					let line = String::from_utf8_lossy(&raw);
					if let Some(cl) = parse_chat_line(line.trim_end()) {
						if let Some(t) = cl.content {
							if !t.is_empty() { yield Ok(t); }
						}
						if cl.done { return; }
					}
				}
			}
			// A `stream:false` response is one JSON object with no trailing newline,
			// so the line-split loop never fires — flush the buffered remainder. It
			// carries both the full content and `done:true`; emit content first.
			if !buf.is_empty() {
				let line = String::from_utf8_lossy(&buf);
				if let Some(cl) = parse_chat_line(line.trim_end()) {
					if let Some(t) = cl.content {
						if !t.is_empty() { yield Ok(t); }
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
			// No runtime or a completion error both collapse to "" — the distill /
			// edge-label callers treat that as "no output".
			block_on_in_place(client.complete(&prompt)).and_then(Result::ok).unwrap_or_default()
		}
	}
}

/// Drive an async `Client` call to completion from a synchronous context that is
/// itself inside the multi-thread runtime. `block_in_place` hands this worker
/// thread back to the scheduler while we block, so `block_on` is legal here —
/// plain `block_on` on a runtime worker panics ("Cannot start a runtime from
/// within a runtime"). `None` when called outside any runtime; the caller maps
/// that to its own empty/error result.
pub(crate) fn block_on_in_place<F: std::future::Future>(fut: F) -> Option<F::Output> {
	let handle = tokio::runtime::Handle::try_current().ok()?;
	Some(tokio::task::block_in_place(|| handle.block_on(fut)))
}

/// Map a non-2xx response to `LlmError::Api` (reading the error body for the
/// message), passing a successful response through unchanged so the caller can
/// `.await?` then parse it. Shared by every embed/reason request path.
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
	embeddings: Vec<Vec<f64>>,
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

/// Context window for the answer model's `/api/chat` load. The `/ask` path glues
/// already-retrieved graph nodes into a short paragraph — it never needs a large
/// window, and Ollama's default 32k allocates a KV cache big enough to spill a 4b
/// model off an 8 GB GPU onto CPU (~2x slower, and it evicts the embedder). 8192
/// keeps qwen3.5:4b fully GPU-resident alongside the embedder.
const ANSWER_NUM_CTX: u64 = 8192;

/// Keep the answer model resident between requests. `/v1` ignores `keep_alive`;
/// `/api/chat` honors it. Paired with the ~4-min warm ping this holds the model
/// so a user `/ask` never pays a cold reload.
const ANSWER_KEEP_ALIVE: &str = "10m";

/// Context window for the embedder's `/api/embed` load. Retrieval embeddings are
/// short (paragraph chunks, query strings), but Ollama otherwise allocates the
/// model's DEFAULT context — 32k for qwen3-embedding — whose KV cache balloons a
/// 0.6b model to ~5.8 GB of VRAM. That cannot share an 8 GB GPU with the answer
/// model, so the two thrash on every `/ask` and wedge under concurrent load.
/// 2048 holds the embedder at ~1.5 GB, leaving the answer model resident too.
const EMBED_NUM_CTX: u64 = 2048;

/// Keep the embedder resident between requests, same rationale as the answer
/// model: `/api/embed` honors `keep_alive`, the ~4-min warm ping re-touches it,
/// and retrieval never pays a cold embedder reload.
const EMBED_KEEP_ALIVE: &str = "10m";

/// Context window for the CPU-bound reason model. Distillation/edge-proposal
/// prompts are bounded (chunks, capture deltas), and a larger window only slows
/// CPU prefill, so cap it modestly.
const REASON_NUM_CTX: u64 = 8192;

/// The reason model runs on CPU and is used in bursts; a short keep-alive frees
/// the (large) CPU model's RAM between distillation runs rather than pinning it.
const REASON_KEEP_ALIVE: &str = "2m";

/// Host / port substrings that mark an endpoint as a local Ollama server. The
/// loopback names plus Ollama's default port. NOTE: a Docker-bridged or
/// non-standard-port Ollama (e.g. `http://ollama:11434` resolves the port, but a
/// remapped `:8080` would not) is still classed as cloud and routed to `/v1`.
/// Making this configurable per-endpoint is deferred — it needs a flag on
/// [`Endpoint`] threaded from config, not just a constant.
const OLLAMA_LOCAL_MARKERS: [&str; 3] = ["localhost", "127.0.0.1", ":11434"];

/// Heuristic: is this endpoint a local Ollama server? Only Ollama honors the
/// native `options.num_gpu`/`num_ctx`; cloud endpoints (OpenAI-compat) must stay
/// on the `/v1` path. URLs are already normalized (trailing `/` and `/v1`
/// stripped) before storage, so match on the loopback host / default port.
fn is_local_ollama(url: &str) -> bool {
	OLLAMA_LOCAL_MARKERS.iter().any(|m| url.contains(m))
}

/// One parsed line of an Ollama `/api/chat` response. `content` is the message
/// delta — present on token chunks, typically empty on the terminal chunk.
/// `done` marks the final object. A `stream:false` response is a SINGLE object
/// carrying both the full content AND `done:true`, so a consumer must emit
/// `content` before acting on `done`, or the whole answer is lost.
#[derive(Debug, PartialEq)]
struct ChatLine {
	content: Option<String>,
	done: bool,
}

/// Parse one line of an Ollama `/api/chat` response — NDJSON when streaming, a
/// single object when not (`{"message":{"content":"…"},"done":bool}`). Blank
/// lines / parse failures → `None`.
fn parse_chat_line(line: &str) -> Option<ChatLine> {
	if line.is_empty() {
		return None;
	}
	let v: Value = serde_json::from_str(line).ok()?;
	let content = v
		.get("message")
		.and_then(|m| m.get("content"))
		.and_then(Value::as_str)
		.map(str::to_string);
	let done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
	Some(ChatLine { content, done })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chat_line_yields_token_then_done() {
		// Streaming token chunk: content present, not done.
		assert_eq!(
			parse_chat_line(r#"{"message":{"role":"assistant","content":"He"},"done":false}"#),
			Some(ChatLine { content: Some("He".to_string()), done: false })
		);
		// Streaming terminal chunk: empty content, done.
		assert_eq!(
			parse_chat_line(r#"{"message":{"content":""},"done":true,"done_reason":"stop"}"#),
			Some(ChatLine { content: Some(String::new()), done: true })
		);
		// `stream:false` single object: full content AND done in one line — both
		// must survive so the caller emits the answer before stopping.
		assert_eq!(
			parse_chat_line(r#"{"message":{"content":"Full answer."},"done":true}"#),
			Some(ChatLine { content: Some("Full answer.".to_string()), done: true })
		);
		assert_eq!(parse_chat_line(""), None);
		assert_eq!(parse_chat_line("not json"), None);
		assert_eq!(
			parse_chat_line(r#"{"message":{},"done":false}"#),
			Some(ChatLine { content: None, done: false })
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

	#[test]
	fn local_ollama_markers_match_loopback_and_default_port() {
		assert!(is_local_ollama("http://localhost"));
		assert!(is_local_ollama("http://127.0.0.1:9999"));
		assert!(is_local_ollama("http://ollama:11434"));
		// A remote OpenAI-compat host is NOT local — must stay on /v1.
		assert!(!is_local_ollama("https://api.openai.com"));
	}

	// -- embed batch-then-single fallback (stub server) --------------------

	async fn serve(app: axum::Router) -> String {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
		format!("http://{addr}")
	}

	/// `/api/embed` distinguishes a batch attempt (array `input`) from the single
	/// retry (string `input`): the embed() fallback fires `embed_single` only
	/// after a retry-worthy batch failure.
	#[tokio::test]
	async fn embed_falls_back_to_single_on_transient_batch_error() {
		use axum::http::StatusCode;
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|body: axum::Json<Value>| async move {
				if body.0["input"].is_array() {
					(StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({ "error": "busy" })))
				} else {
					(StatusCode::OK, axum::Json(serde_json::json!({ "embeddings": [[1.0, 2.0, 3.0]] })))
				}
			}),
		);
		let url = serve(app).await;
		let client = Client::new_embed_only(&url, "m");
		let v = client.embed("hello").await.expect("transient batch -> single retry succeeds");
		assert_eq!(v, vec![1.0, 2.0, 3.0]);
	}

	/// An empty batch response is retry-worthy too (`should_retry_single` matches
	/// `EmptyEmbedding`), so the single retry still runs and recovers.
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
		let url = serve(app).await;
		let client = Client::new_embed_only(&url, "m");
		let v = client.embed("x").await.expect("empty batch -> single retry succeeds");
		assert_eq!(v, vec![9.0]);
	}

	/// A permanent client error (400) fails identically as a single call, so
	/// embed() must propagate it WITHOUT a wasted second round-trip. The hit
	/// counter proves exactly one request was made.
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
					(StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({ "error": "bad model" })))
				}
			}),
		);
		let url = serve(app).await;
		let client = Client::new_embed_only(&url, "m");
		let err = client.embed("hello").await.unwrap_err();
		assert!(matches!(err, LlmError::Api { status: 400, .. }), "permanent error propagates, got {err:?}");
		assert_eq!(hits.load(Ordering::SeqCst), 1, "no wasted single retry on a permanent error");
	}
}
