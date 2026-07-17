# splinter: src/gnn/gcn.rs

Second-pass migration:
- `forward_graph_aggregates_then_projects_to_out_features`: deleted the 3-line preamble. Contract recorded here — the layer is built with no norm/dropout/activation so the output is a pure linear projection of the normalized-adjacency-aggregated features; it asserts the output shape AND that `last_norm_adj` / `last_pre_act` are cached by `forward_graph` alone, independent of ever running backward.
- `try_backward_before_forward_is_missing_state_and_infallible_path_zeroes`: deleted the narration on the infallible delegate (the assert message carries it). Kept the 1-line note that the activation is what makes the `last_pre_act` guard trip first.
- Collapsed a stray blank line left inside `backward_graph`'s error arm by the first pass.
- Kept inline: the `try_backward_graph` fallible/infallible contract, and the dInput shape derivation (`in_features == linear.weight.rows`) in the error arm.
