# src/retrieval/pagerank.rs — commentary

- `pagerank`: the seed-teleport personalization follows HippoRAG 2 — teleport at query-linked entities for multi-hop / associative recall.
- `pagerank` adjacency: `g.entity_adjacency()` is epoch-cached on the graph — retrieval runs many queries between writes, so the rebuild (String clones for every entity and edge) happens once per mutation, not once per query.
- Convergence: power iteration on a stochastic matrix converges geometrically, so a well-connected graph settles far under the `iters` cap; the L1 delta vs CONVERGENCE_EPS detects the fixed point.
- Top-k: `select_nth_unstable_by` partitions in O(n) average instead of O(n log n) full sort, then only the k survivors are sorted. pagerank runs per query over the entire entity graph, so this matters as the graph grows toward Qdrant scale.
PageRank over the entity graph:
- teleport_vector: seed scores (clamped >= 0) normalized to sum 1; no usable seeds falls back to a uniform teleport = global (non-personalized) PageRank. Non-empty seeds personalize it (query-personalized).
- The iteration loop stops early once the rank vector stops moving (L1 delta < 1e-9); `iters` is just an upper bound.
- Dangling mass (nodes with no out-edges) is redistributed along the TELEPORT vector, not uniformly, so the personalization bias is preserved rather than leaked. Correctness-critical — a "fix" to uniform breaks personalization.
- Final ordering is score desc, id asc. Unique ids make the comparator a STRICT total order, so the top-k partition (select_nth_unstable_by) + sorting only survivors equals a full sort + take.
- Self-loops (from == to) are dropped during adjacency build, so they don't inflate a node's rank.
