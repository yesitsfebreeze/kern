# splinter: src/retrieval/hyde.rs

Second-pass migration:

- `empty_hypo_vec_returns_query_without_fusing` contract (moved out of the test body): the dimension-0 edge is `embed` returning `Ok(vec![])` rather than an `Err`. The `hypo_vec.is_empty()` half of the guard in `expand_query` is what catches it — the `hypo_vec.len() != query_vec.len()` half alone would not, since a 0-d query vector would compare equal. Without the empty check the fusion `zip` would yield nothing and `expand_query` would return an empty vector instead of the original query, silently destroying the query embedding for the rest of retrieval.
- `expand_query` is fail-open throughout: HyDE disabled, a query at/over `hyde_min_query_tokens`, a zero-token query, a missing LLM or embedder, a blank hypothesis, an embed `Err`, and a length-mismatched or empty hypothesis vector all return `query_vec` unchanged. HyDE only ever fires for short queries — the token floor exists because a long query already carries enough signal.
- Fusion is `q*(1-w) + h*w` followed by L2 normalization, so `hyde_fusion_weight = 1.0` drops the query component entirely and leaves the pure hypothesis direction (`fusion_weight_one_yields_pure_hypo_direction`). The normalization is what keeps the fused vector on the same cosine scale as the un-expanded query.
