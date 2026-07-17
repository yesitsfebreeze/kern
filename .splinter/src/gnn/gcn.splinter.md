# splinter: src/gnn/gcn.rs

Second-pass migration:
- `forward_graph_aggregates_then_projects_to_out_features`: deleted the 3-line preamble. Contract recorded here — the layer is built with no norm/dropout/activation so the output is a pure linear projection of the normalized-adjacency-aggregated features; it asserts the output shape AND that `last_norm_adj` / `last_pre_act` are cached by `forward_graph` alone, independent of ever running backward.
- `try_backward_before_forward_is_missing_state_and_infallible_path_zeroes`: deleted the narration on the infallible delegate (the assert message carries it). Kept the 1-line note that the activation is what makes the `last_pre_act` guard trip first.
- Collapsed a stray blank line left inside `backward_graph`'s error arm by the first pass.
- Kept inline: the `try_backward_graph` fallible/infallible contract, and the dInput shape derivation (`in_features == linear.weight.rows`) in the error arm.

## Preserved from stripped comments (2026-07-17)
- `with_rng` exists for deterministic weight init from a seeded RNG — use in tests asserting on training dynamics so runs don't depend on system entropy.
- Backward guard order (non-obvious): when the layer has an activation, the activation path's `last_pre_act` missing-state guard trips before the `last_norm_adj` guard. Tests that want to hit the missing-state path construct the layer WITH an activation.
