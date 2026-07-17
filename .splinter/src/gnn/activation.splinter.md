# src/gnn/activation.rs — commentary

- `Activation` (enum with paired `deriv`) replaced bare `fn(f64) -> f64` activations whose backward used a central finite difference (EPS 1e-5) — that approximation blurred kink derivatives (ReLU at x≈0 gave ~0.5) and cost two forward evaluations per element.
- GELU: the tanh approximation was chosen because Rust std has no erf primitive and it is the GPT/BERT formulation used by modern GNN/Transformer stacks in practice.

Second-pass migration:
- `Activation` doc trimmed to the contract (analytic `deriv`, never a finite difference); the "biased at kinks, slower" rationale is already recorded above.
- `relu_deriv_is_exact_at_and_near_kink`: deleted narration ("no finite-difference smear"); the test name plus the exact-equality asserts carry it.
- Kept inline: the GELU tanh-approximation contract (`gelu_deriv` is the exact derivative OF the approximation, so forward/backward stay consistent) and the `SQRT_2_OVER_PI` magic-number label.
