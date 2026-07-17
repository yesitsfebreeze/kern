# src/bench_support/ndcg.rs — commentary

- `recall_at_k`: coverage (not ordering) is the headline metric for the Qdrant-parity bench — aspiration.md Tier-0.

Second-pass migration (test derivations moved out of the source):
- NDCG formula: gain is binary (1 iff the id is in `expected_ids`), discount is `1/log2(rank+2)` for 0-based rank. `NDCG = DCG/IDCG`, and IDCG is the DCG of `min(|expected|, k)` hits placed at the top. Both `k` and `|expected|` cap IDCG, so a top-1 relevant result at `k=1` scores a perfect 1.0 (`k_caps_both_dcg_and_idcg`).
- `partial_hit_matches_the_formula` worked example: ranked `[a, x, b]`, expected `{a, b}`, k=3. DCG = 1/log2(2) + 1/log2(4) = 1.0 + 0.5 = 1.5 (a@0, b@2). IDCG = 1/log2(2) + 1/log2(3) = 1.0 + 0.63093 = 1.63093 (2 ideal hits).
- `recall_at_k` is order-insensitive coverage and both sides are de-duplicated, so a repeated relevant id cannot inflate recall past 1.0 (`recall_is_bounded_by_k_and_never_exceeds_one`). When `|expected| > k`, 1.0 is unreachable — that trap stays inline.
