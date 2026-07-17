# src/ingest/split.rs — commentary

Second-pass migration (from source comments):

- `split` fallback semantics: if no `llm` is given, OR the LLM returns an empty response (`llm_split` maps an empty response to an empty `Vec`), `split` silently falls back to `paragraph_split` (blank-line paragraph chunking) so ingest always gets *some* chunking instead of failing. Whitespace-only input yields no chunks at all — no bogus blank statement is produced.
- `paragraph_split` final branch: a single blank-line-free paragraph never reaches it (it survives as one chunk from `trim_nonempty`); the branch is only reached for whitespace-only input, and emits nothing rather than a blank/whitespace chunk that downstream would have to filter anyway.
- `llm_split`: an empty `descriptor` omits the "This text describes …" clause entirely (covered by `llm_split_folds_descriptor_into_the_prompt`).
