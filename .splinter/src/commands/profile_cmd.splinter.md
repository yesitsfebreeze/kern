# src/commands/profile_cmd.rs — commentary

Second-pass migration (rationale moved out of the source comments):

- cold/warm embed split: the first embed may pay an Ollama model (re)load; the second is the steady-state cost every later stage actually sees.
- LLM stage wiring: one complete-closure is shared by the profiled query and distill (it was two `complete_func()` calls); the embed closure comes from the shared `commands::embed_fn` factory.
- `cmd_profile_no_llm_path_does_not_panic` contract: the no-LLM path must run end-to-end without panicking on an empty graph — load → cold/warm embed → vector search → the three no-LLM query modes → digest build. `no_llm=true` means reason/answer are never touched, so only Ollama-native `/api/embed` is stubbed (any input → a fixed 3-dim embedding); everything downstream runs on a fresh empty graph in a temp data dir, which also gives `Store::open` a real directory to bind.
cmd_profile is read-only (nothing persisted) so it is safe to run next to a live daemon. DISTILL_SAMPLE is a small fixed conversation so distill timing is comparable across runs. The two embeds are labeled cold/warm because the first embed may pay a model (re)load and the second is the steady-state cost. The no_llm test stubs only /api/embed — no_llm=true never touches reason/answer.
