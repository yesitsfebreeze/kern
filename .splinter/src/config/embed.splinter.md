# src/config/embed.rs — commentary

- `DEFAULT_EMBED_MODEL` = qwen3-embedding:0.6b, chosen because it is small (~640 MB), fast, and higher retrieval quality than nomic/mxbai (tops MTEB for its class).

Second-pass migration:
- `DEFAULT_EMBED_URL` / `DEFAULT_EMBED_MODEL` are the single source of truth shared by `EmbedConfig::default` and the CLI `--embed-url` / `--embed-model` clap defaults; `default_uses_the_shared_constants` guards against the two silently drifting apart.
- `EmbedConfig::model` dimension lock, full consequence: the vector dimension is fixed into the graph on first ingest. Switching models on an existing store without `kern reembed` leaves the new dimension mismatching stored vectors, and search then silently MISSES rather than erroring — that silence is why the inline comment is kept.
- `EmbedConfig::url`: local Ollama uses the native `/api/embed` path; a `/v1` suffix routes to the OpenAI-compat `/v1/embeddings` endpoint (cloud, vLLM).
