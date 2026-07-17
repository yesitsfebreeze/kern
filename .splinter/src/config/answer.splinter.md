# src/config/answer.rs — commentary

- `AnswerConfig` was split from `ReasonConfig` on purpose: the two have OPPOSITE optimization targets. Distillation/edge-proposal ([reason]) runs in the background — latency is free, structured-output reliability matters, so it wants a bigger model. The answer path is user-facing and only glues already-retrieved graph nodes into prose, so it wants the smallest model that clears the grounding floor. Keeping them one knob forced one side to lose.
- `DEFAULT_ANSWER_MODEL` = qwen3.5:4b: dumb-fast glue over graph context; fits 8 GB VRAM alongside the 0.6b embedder with KV headroom; same family as the embedder so it shares Ollama's tokenizer cache.

Second-pass migration:
- The empty `url`/`key` defaults are intentional, not an oversight: `Config::answer_url` / `answer_key` then fall back to `[reason]`, which falls back to `[embed]`, so a single local Ollama needs no extra wiring. `default_leaves_endpoint_empty_and_uses_the_answer_model` pins that contract.
