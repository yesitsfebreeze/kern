# src/config/retrieval.rs — commentary

- `mmr_lambda` = 0.75: relevance-dominant (standard MMR region ~0.7). The previous 0.45 over-diversified: on the workload trace it suppressed legitimate multi-relevant cluster hits below rank 10 (recall@10 0.925 -> 1.0, NDCG@10 0.928 -> 0.999 at 0.75; `just bench-workload`, sweep 2026-07-16). Ingest-time dedup (cosine 0.95) already handles true duplicates, so MMR need not.
- `bm25_k1`/`bm25_b`: these fields were dead (unwired) for a while; the range checks in `validate` were added when they were wired into the lexical index, so bad values are caught at config load rather than silently clamped at query time.

Second-pass migration:
- `validate` (`rrf_k` / `seed_k` / `max_deliver_results` group): these are checked because an out-of-range value silently BREAKS retrieval — there is no graceful fallback and no valid use of the bad value. Contrast `important_min_cosine > 1.0`, which is deliberately left unchecked because it is a legitimate "disable" idiom. That distinction is the reason the validate list is selective rather than exhaustive.
