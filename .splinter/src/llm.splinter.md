# src/llm.rs — commentary

Two dispatch paths per leg: local Ollama native `/api/*` (supports `num_ctx`/`keep_alive`/`num_gpu`) vs OpenAI-compat `/v1/*` (cloud, vLLM, llama-server). Local URLs default native; an explicit `/v1` suffix forces compat so a local vLLM at `http://localhost:8000/v1` is not mistaken for Ollama.

- `should_retry_single`: propagating permanent errors (400/401) lets ingest's `embed_with_retry` short-circuit rather than pay a second HTTP round-trip per chunk.
- `Inner.temperature`: eval pins the judge to 0.0 because the judge is the measurement instrument — its verdicts must not carry sampling noise.
- `embed_body` / `EMBED_NUM_CTX`: full VRAM story — without a num_ctx cap Ollama allocates a KV cache for the model's DEFAULT context (32k for qwen3-embedding), ballooning a 0.6b embedder to ~5.8 GB, which cannot coexist with the answer model on an 8 GB GPU; every `/ask` (embed → answer) thrashes Ollama swapping the two, and under the multi-daemon forest's concurrent load wedges it outright. 2048 holds the embedder at ~1.5 GB. `Client::answer`'s `ANSWER_NUM_CTX` is the mirror on the answer path.

Second-pass migration (from source comments):

- `embed_propagates_permanent_batch_error_without_retry`: the test's hit counter is the oracle — it proves exactly one HTTP request is made on a permanent (400) error, i.e. no wasted single retry.
