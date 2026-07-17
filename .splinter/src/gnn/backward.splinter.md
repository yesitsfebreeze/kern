# src/gnn/backward.rs — commentary

- `act_deriv_mul`: replaced a central finite-difference of the activation — the analytic derivative is exact at kinks (the old approach leaked ~0.5 gradient for ReLU at x=±1e-6) and costs half the activation evaluations. Pinned by test `relu_backward_is_exact_no_kink_bias`.
- `row_sum_sq`: shared by the L2-norm forward and backward passes so the per-row reduction lives in one place.
- `assert_input_grad_matches_numeric` (test): exists because the param-gradient check never exercises the input gradient, and every `model.rs` test is single-layer — without it the layer-to-layer gradient flow (`Model::backward` chaining `d_input` into the previous layer's `d_out`) was unverified.
- `gnn_math_tests` module was added as purely additive coverage (no production behaviour change).


Second-pass migration:
- `l2_norm_backward` inline math compressed. Full statement: tangent-space projection dL/dx = (d_out − x̂·(d_out·x̂))/‖x‖ with x̂ = x/‖x‖; the projection `(I − x̂x̂ᵀ)/‖x‖` is easy to get subtly wrong (dotting with raw `pre_norm` instead of x̂ drops a 1/‖x‖ factor and inflates the gradient), which is why `l2_norm_backward_matches_numeric_gradient` pins it against a central finite difference (loss = sum(l2_normalize_rows(x)), d_out all-ones, tolerance 1e-4 relative).
- `l2_norm_backward_zero_row_yields_zero_grad` essay deleted; contract: a zero row has no defined direction, forward and backward both skip it, so its gradient stays zero and no NaN arises from a 1/0 norm.
- `row_sum_sq` doc deleted (name says it; sharing rationale already noted above).
- `relu_backward_is_exact_no_kink_bias` setup narration compressed; inputs straddle zero with grad all 1.0.

## Preserved from stripped comments (2026-07-17)
- `act_deriv_mul` multiplies the incoming gradient by the activation's ANALYTIC derivative at the pre-activation values — exact, no finite-difference bias at kinks (matters for ReLU/LeakyReLU at 0).
- `GraphLayer::set_training` default flips only the layer's dropout — the sole train-mode-sensitive component these layers carry.
- Test methodology (gnn_math_tests): analytic grads are taken with `d_out = ones`, loss = `sum(output)`, checked against CENTRAL finite differences (±H, H=1e-6), init-agnostic.
- Gradient identity verified: GCN input (layer-chaining) gradient is `d_input = Aᵀ·(d_out·Wᵀ)` — the gradient `Model::backward` chains into the previous layer's `d_out`.
