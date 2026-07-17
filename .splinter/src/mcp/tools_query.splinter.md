# src/mcp/tools_query.rs — commentary

- `tool_schemas`: schema is co-located with the `tool_query` handler so the two can't silently drift; aggregated by `tools::tool_definitions`.
- `tool_query` (answer cache): only the answer path is worth caching — it fires HyDE + synthesis (tens of seconds); pure vector retrieval is already sub-millisecond. Only unfiltered, default-sorted queries are cacheable because a filter or non-default sort changes the result set/order while the query vector stays the same (enforced by `query_is_cacheable`). The cache `tag` is the retrieval mode, keeping the three modes from colliding on one entry.
- `base_entity_json`: defined once so the kern_rpc-consumed contract has a single source of truth — the envelope tests build on this same fn instead of a hand-mirrored copy that could silently drift. The kind/scheme/status echo exists so `kern_rpc::query` can build `EntityRef` without a second graph lookup (landed as Slice Z, along with `envelope_shape_tests`).
Design notes carried from stripped comments:
- parse_time_filter: empty string = no filter; a non-empty unparseable value is a HARD error, so a typo'd time filter fails loudly instead of silently going unfiltered.
- QueryArgs.source is a legacy free-form source-system filter; prefer the typed `scheme` (URI scheme on Source, unknown values error). `as_of` is a bi-temporal point query returning the revision whose half-open [valid_from, valid_to) window covered the instant. `include_history` also returns Superseded revisions reachable from active hits.
- answer_llm_args gates the LLM/embedder handles on answer:true; answer:false is a fast pure-vector retrieval (no HyDE/rerank/synthesis).
- query_is_cacheable: only answer-on, cache-enabled, UNFILTERED, default-sorted queries cache — any filter/sort changes the result set/order for the same query vector.
- Cold-tier recall: below k, fill from the cold store read-only so demoted thoughts stay findable without rehydrating into the hot graph; skipped on the exact-text fast path (vec is None).
- History rides alongside the top-k, not displacing it (take_n = k + history_ids.len()).
- Regression guard (answer_gating_tests): the unconditional-LLM bug overran the MCP client timeout as `-32000 Connection closed`. envelope_shape_tests guard that the envelope carries kind/scheme/status labels — kern_rpc::query silently falls back to defaults if a refactor drops them.
