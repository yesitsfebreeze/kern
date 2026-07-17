# src/retrieval/pagerank.rs — commentary

- `pagerank`: the seed-teleport personalization follows HippoRAG 2 — teleport at query-linked entities for multi-hop / associative recall.
- `pagerank` adjacency: `g.entity_adjacency()` is epoch-cached on the graph — retrieval runs many queries between writes, so the rebuild (String clones for every entity and edge) happens once per mutation, not once per query.
- Convergence: power iteration on a stochastic matrix converges geometrically, so a well-connected graph settles far under the `iters` cap; the L1 delta vs CONVERGENCE_EPS detects the fixed point.
- Top-k: `select_nth_unstable_by` partitions in O(n) average instead of O(n log n) full sort, then only the k survivors are sorted. pagerank runs per query over the entire entity graph, so this matters as the graph grows toward Qdrant scale.
