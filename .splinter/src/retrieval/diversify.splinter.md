# src/retrieval/diversify.rs — commentary

- `mmr` optimization: the naive form re-scans every selected item for every candidate each round — O(pool * target^2) cosines. The shipped form precomputes sim_q once (pure function of fixed inputs) and maintains max_sim incrementally (one fold pass per pick), costing O(pool * target). Output is provably byte-identical to the naive form: the per-round argmax over `lambda*sim_q - (1-lambda)*max_sim` is unchanged, max over a set is order-independent for f64, swap_remove keeps the pool's evolving order matching a plain remove-and-rescan, and output order stays the `selected` push-order. The equivalence is pinned by `mmr_is_byte_identical_to_naive_reference` against `mmr_reference` (the pre-optimization body kept verbatim as an oracle — do not simplify it).
- `max_sim` floor at 0.0 matches the old `fold(0.0, max)`; the skip of vector-less chosen items matches the old code's empty-selected branch.
MMR diversify algorithm notes:
- sim_q[i] = sim(query, candidate i), fixed for the whole selection, computed once per candidate; falls back to the incoming score when either vector is absent (query empty or candidate has no vector).
- max_sim[i] = max cosine of candidate i to any already-selected item, FLOORED at 0.0 (negative similarity never rewards); maintained incrementally as items are chosen.
- sim_q and max_sim are swap-removed in lockstep with pool so index i keeps addressing the same candidate as pool[i]. INVARIANT — keep the three swap_removes together.
- When folding a chosen item into remaining candidates' redundancy: a vector-less item contributes 0.0 and can only lose the max, so it is skipped.

Tests:
- mmr_reference is the pre-optimization O(P*T^2) MMR body, kept verbatim as the equivalence oracle. Do NOT "simplify" it — its whole value is being the old code.
- mmr_is_byte_identical_to_naive_reference: random pools deliberately exercise negative cosines (the 0.0 floor), empty vectors (fallback paths), and tied scores (coarse buckets). Equivalence is exact — no float tolerance.
- dedup_preserves_relative_order_of_survivors guards that retain keeps survivors in original positions, so a future rewrite (collect/sort) can't silently reorder the delivered set.
