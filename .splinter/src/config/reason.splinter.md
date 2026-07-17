# src/config/reason.rs — commentary

- `DEFAULT_REASON_URL`: the url was once empty by default, which broke the distill and answer paths for any kern without an explicit `[reason] url` — hence the concrete local-Ollama default.
- `DEFAULT_REASON_MODEL` = qwen2.5:7b: small, fast, reliable structured-output model — which is what distillation / naming / edge-proposal need.

Second-pass migration:
- Const doc comments dropped from source; rationale for both `DEFAULT_REASON_URL` and `DEFAULT_REASON_MODEL` is the note above. `DEFAULT_REASON_URL` is deliberately the same local Ollama that serves embeddings (`config::DEFAULT_EMBED_URL`), and the consts are public so callers can reference the baseline without constructing a full `ReasonConfig`.
- `ReasonConfig::key` mirrors `EmbedConfig::key` (Bearer token, empty = unauthenticated local Ollama).
Endpoint/key precedence: reason.url/key empty -> falls back to the embed endpoint (via Config::reason_url/reason_key accessors). answer.* falls back to the resolved reason.*.

DEFAULT_REASON_URL points at the same local Ollama that serves embeddings (DEFAULT_EMBED_URL). It exists because empty-by-default previously broke the distill and answer paths for any kern without an explicit [reason] url.

Default model qwen2.5:7b: small, fast, reliable structured-output model — what distillation / naming / edge-proposal need. Const is exposed so callers can reference the baseline without building a full ReasonConfig.
