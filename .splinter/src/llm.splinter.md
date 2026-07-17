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
