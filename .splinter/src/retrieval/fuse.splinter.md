# src/retrieval/fuse.rs — commentary

- `rrf`: the deterministic id tiebreak exists to keep recall reproducible across runs and tests deterministic (equal-score entities previously followed HashMap iteration order). The `select_nth_unstable_by` partition is a perf choice: only top_k of a potentially large fused union is delivered, so partition in O(n) average instead of fully sorting all n in O(n log n).
Second-pass migration:

- `rrf` weighting rationale (moved off the doc comment): the point of the per-list weight is to down-weight *query-independent* lists. In `answer::fuse_hybrid_seeds` the dense and lexical lists are query-relevant and weigh 1.0, while the importance and PageRank priors are query-blind and get `cfg.rrf_global_weight`. At equal weight a globally popular but irrelevant entity sitting at rank 1 of the importance list contributes exactly as much as a genuinely relevant entity at rank 1 of the dense list, so a prior could match a real hit; the weight is what keeps priors as tie-breakers rather than drivers. Pinned by the 0.5-global-weight test that requires dense `rel` to outrank `pop` when both are rank 1 in their own list.
- A missing weight defaults to 1.0, so `rrf` with an empty/short `weights` slice degrades to plain unweighted RRF.
