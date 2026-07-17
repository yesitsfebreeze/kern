# src/llm.rs — commentary

Two dispatch paths per leg: local Ollama native `/api/*` (supports `num_ctx`/`keep_alive`/`num_gpu`) vs OpenAI-compat `/v1/*` (cloud, vLLM, llama-server). Local URLs default native; an explicit `/v1` suffix forces compat so a local vLLM at `http://localhost:8000/v1` is not mistaken for Ollama.

- `should_retry_single`: propagating permanent errors (400/401) lets ingest's `embed_with_retry` short-circuit rather than pay a second HTTP round-trip per chunk.
- `pins_reason_to_cpu`: the `num_gpu:0` reason pin is CONDITIONAL, not unconditional. Measured mechanism behind it (2026-07-17): Ollama does NOT start a second runner when the same model tag arrives with a different `num_gpu` — the first placement wins and later calls silently reuse it. So with reason and answer on one model, an unconditional pin would strand the shared runner on the CPU and make every `/ask` pay CPU inference. The pin's only justification was a *distinct, larger* reason model evicting the answerer from an 8 GB card; one model means nothing to evict. Verified end-to-end: stock config now loads granite4:3b at 100% GPU for both legs, where the same call previously ran 100% CPU. Splitting `[answer] model` from `[reason] model` re-arms the pin automatically — that is the intended behavior, not a bug.
- `Inner.temperature`: eval pins the judge to 0.0 because the judge is the measurement instrument — its verdicts must not carry sampling noise.
- `embed_body` / `EMBED_NUM_CTX`: full VRAM story — without a num_ctx cap Ollama allocates a KV cache for the model's DEFAULT context (32k for qwen3-embedding), ballooning a 0.6b embedder to ~5.8 GB, which cannot coexist with the answer model on an 8 GB GPU; every `/ask` (embed → answer) thrashes Ollama swapping the two, and under the multi-daemon forest's concurrent load wedges it outright. 2048 holds the embedder at ~1.5 GB. `Client::answer`'s `ANSWER_NUM_CTX` is the mirror on the answer path.

Second-pass migration (from source comments):

- `embed_propagates_permanent_batch_error_without_retry`: the test's hit counter is the oracle — it proves exactly one HTTP request is made on a permanent (400) error, i.e. no wasted single retry.

# Ratings — scope: src/llm.rs

Scope rating: 8/10 — single Client serves reason/answer/embed, Ollama-native + OpenAI-compat routing, streaming, batch-with-single-fallback, serving/eval pin split. Tight test coverage on routing + pins + retry. The `is_local_ollama` host check was too loose (substring match); tightened to `//`-anchored authority match.

## Function ratings

- `Client::new` — 8/10: stores three endpoints, builds reqwest client. Clean.
- `Client::answer` — 8/10: streaming SSE parser, native + compat paths, `drain_stream_lines` for incremental parse. Correct timeout handling.
- `Client::complete` — 8/10: one-shot completion with native/compat routing + serving pin.
- `Client::embed` — 9/10: batch-first with single-fallback on transient/empty — correct fail-open.
- `Client::embed_batch` / `embed_single` — 8/10: batch endpoint with fallback.
- `is_local_ollama` — 7/10→9/10: was substring match (`localhost` matched `notlocalhost.com`); fixed to `//localhost` / `//127.0.0.1` authority-anchored. The `:11434` port marker stays loose (WSL gateway heuristic, acceptable).
- `wants_native` — 9/10: clean — `/v1` suffix opts into compat, else `is_local_ollama`.
- `pins_reason_to_cpu` — 9/10: correct conditional — pins only when reason != answer model/endpoint.
- `Client::for_eval` — 9/10: flips pin to GPU + seeds sampling for eval. Correct separation.
- `Client::with_temperature` — 9/10: judge pin to 0. Clean.
- `is_transient` / `should_retry_single` — 8/10: correct transient classification for retry logic.
- `parse_sse_delta` / `parse_chat_line` — 8/10: SSE delta parser, handles both backends.
- `drain_stream_lines` — 8/10: incremental line-buffered stream parser. Correct.
- `block_on_in_place` — 7/10: blocks on async from sync context (MCP tool path). Necessary for the sync dispatch surface; documented limitation.
- `local_ollama_markers_match_loopback_and_default_port` — 9/10: covers loopback/default-port/remote + the new `notlocalhost` false-positive guard.
## Design context (moved from source doc comments)

- Module: provider-agnostic LLM dispatch — `embed`, `complete`, and `answer` legs, each with a native Ollama path and an OpenAI-compat path (`wants_native` picks).
- `should_retry_single`: retry a failed batch embed as a single ONLY for batch-specific failures (transient / empty batch) — a permanent client error fails identically as a single, so no wasted second round-trip.
- `AnswerParams.num_predict` caps generated tokens: `None` for a real answer, `Some(1)` for the warm ping.
- `Endpoint`: empty fields fall back per `Client::new` (answer/embed -> reason), so `Endpoint::default()` means "reuse the reason endpoint".
- `Inner.reason_gpu`: serving pins the reason model to CPU (`num_gpu:0`) so a distillation burst can't evict the embedder + answer model; eval flips this. `Inner.seed`: sampling seed for reason calls (both paths); eval sets it, serving doesn't. `Inner.temperature`: reason-call temperature override; eval pins the judge to 0.0.
- `Client::for_eval`: eval-mode client — reason calls may use the GPU (they ARE the workload) and sampling is seeded so multi-seed runs are reproducible.
- `Client::new`: empty answer/embed fields fall back to reason (embed: url+key; answer: url+key+model — an empty answer model would 400 on `/ask`, which is why answer falls back but embed doesn't take a reason model).
- `embed_body`: `/api/embed` body; `input` is one string or an array. Only the NATIVE endpoint honors `num_ctx`/`keep_alive`; `truncate` clips instead of erroring.
- `post_checked`: POST body as JSON, mapping any non-2xx to `LlmError::Api`. Body decoding stays with the caller — each path parses a different shape.
- `Client::answer`: answer-model entry point (`/ask` UI, `query --answer`, warm ping). Yields each non-empty content delta in order; errors surface as a single `Err`.
- `complete_func`: no runtime or a completion error both collapse to "" — the distill / edge-label callers treat that as "no output".
- `NativeEmbedResponse`: Ollama native `/api/embed` — `embeddings` preserves request `input` order (one row per input string). `OpenAiEmbedResponse`: `/v1/embeddings` order is NOT guaranteed; callers must sort by `index`.
- `wants_native`: local URLs get Ollama native `/api/*`; an explicit `/v1` suffix opts into OpenAI-compat regardless of host (vLLM, llama-server).

### `pins_reason_to_cpu` — full derivation (tightened in source)

Whether serving should force reason calls onto the CPU (`num_gpu:0`).

The pin exists for ONE reason: a distillation burst on a reason model that is a *different, larger* model than the answerer evicts the embedder + answer model from an 8 GB GPU and thrashes `/ask`. When reason and answer resolve to the same model on the same endpoint there is nothing to evict — one Ollama runner serves both legs — so the pin has no justification left and is actively harmful: Ollama keys a runner by its placement, the first call wins, and a reason call would strand the shared runner on the CPU where every subsequent `/ask` then pays CPU inference (measured: same tag, `num_gpu:0` first, and the later GPU-allowed call silently reuses the CPU runner rather than starting a second one). Same model tag on DIFFERENT hosts is two runners on two machines, so the shared-runner reasoning does not apply and the pin stays. Eval always clears the pin: there reason calls ARE the workload.

### LLM const rationales

- `ANSWER_NUM_CTX = 8192`: Ollama's 32k default allocates a KV cache big enough to spill the answer model off the GPU; 8192 keeps it GPU-resident. (kept in source)
- `ANSWER_KEEP_ALIVE = "10m"`: keep the answer model resident (`/v1` ignores `keep_alive`); paired with the ~4-min warm ping so a user `/ask` never pays a cold reload. (kept in source)
- `EMBED_NUM_CTX = 2048`: without a cap Ollama allocates the model's DEFAULT-context KV cache, which cannot share an 8 GB GPU with the answer model. (kept in source)
- `EMBED_KEEP_ALIVE = "10m"`: keep the embedder resident; same rationale as ANSWER_KEEP_ALIVE. (kept in source)
- `REASON_NUM_CTX = 8192`: reason prompts are bounded and a larger window only slows CPU prefill.
- `REASON_KEEP_ALIVE = "2m"`: short keep-alive frees the large CPU model's RAM between distillation bursts.
- `LLM_TIMEOUT = 600s`: overrides the client's 120 s default — slow CPU inference, large RAG prompts, or long streaming answers can run well past it. (kept in source)
- `EMBED_TIMEOUT = 120s`: pinned so embed timeouts stay stable if the client-level default changes.

### Streaming / test context

- `ChatLine`: one parsed streaming event from either backend. The terminal chunk may carry both content and `done:true` — emit content BEFORE acting on `done`. (kept in source)
- Kept in source: `/api/embed` preserves input order (no index sort needed) vs OpenAI must sort by index; `wants_native` reads the pre-normalize URL because `normalize` strips the `/v1` that marks an OpenAI-compat server; the short connect timeout makes a dead endpoint fail fast (transient -> retry) because WSL passthrough to a closed port hangs rather than refusing; the explicit fn-ptr in `answer` unifies both arms (closures have distinct anonymous types); `block_on_in_place` uses `block_in_place` so `block_on` is legal on a runtime worker (plain `block_on` panics), `None` outside any runtime.
- Test intents: permanent client errors (400/401) fail identically as a single call, no wasted retry; a remote OpenAI-compat host is NOT local and must stay on `/v1`; the `is_local_ollama` host substring must be anchored to the authority (`notlocalhost.com` is NOT localhost); local vLLM/llama-server `/v1` opts out of the native path; distinct reason/answer models keep the serving CPU pin; a shared model (incl. the stock zero-config path where answer falls back to reason) drops it; same tag on different hosts keeps it; eval never pins; the embed stub distinguishes batch (array `input`) from the single retry (string), so the fallback fires only after a retry-worthy batch failure; an empty batch response is retry-worthy too.
