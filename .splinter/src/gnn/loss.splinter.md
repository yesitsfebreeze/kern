# splinter: src/gnn/loss.rs

Second-pass migration:
- `link_prediction_grad_matches_numerical_gradient`: deleted the preamble (test name carries it). Method recorded here: central finite differences with H = 1e-6 over every embedding element, compared with a RELATIVE tolerance (`den = max(1, |analytic|, |numeric|)`, ratio < 1e-4) so the check does not go brittle where the gradient is large.
- `link_prediction_aligned_positive_edge_has_lower_loss_than_opposed`: deleted the `// dot +9` / `// dot -9` trailing labels — the fixtures are `[3,0]` vs `[3,0]` (dot +9) and `[3,0]` vs `[-3,0]` (dot -9); the assert message states the ordering being pinned.
- Not inline: the `1e-10` addend inside both `ln()` calls guards `log(0)` when a dot saturates sigmoid to 0 or 1; the loss and grad are both mean-reduced over `pos + neg` count, and `total == 0` returns zero loss / a zeros grad rather than NaN.
