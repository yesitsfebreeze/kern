# src/gnn/graph.rs — commentary

- `adjacency_matrix` is private on purpose: only `normalized_adjacency` needs it, and exposing the O(N^2) dense materialization would invite external misuse on the sparse graphs kern actually builds. A `degree_matrix` helper was removed as dead code.

Second-pass migration:
- `add_self_loops_is_idempotent`: deleted the `// re-running must add nothing` label (test name + assert message carry it).
- `normalized_adjacency_rows_sum_to_one_on_a_regular_graph`: deleted the "fully bidirectional triangle" preamble — the `_on_a_regular_graph` suffix says it. Kept the `// each node now has degree 3` label, which is what makes the row-sum-1.0 expectation checkable.
- Why rows sum to 1 only here (not inline): symmetric normalization is D^-1/2 · A · D^-1/2, whose rows sum to 1 ONLY when every degree is equal — the regular triangle plus self-loops is the fixture that makes that true. Do not generalize the assert to an arbitrary graph.
