# src/gnn/optim.rs — commentary

- `SGD::step` momentum branch: single pass that updates `velocity[j]` and immediately applies it — replaced a second loop that re-indexed `self.velocity[i]` for every element. Independence per parameter is pinned by test `sgd_momentum_velocity_is_independent_per_parameter`.
- `Adam::step`: bias-correction denominators are hoisted out of the element loop because they are constant per step (avoids a `powf` per parameter element).

Second-pass migration:
- `Optimizer::zero_grad` doc compressed to 2 lines (it zeroes the PASSED-IN tensors, not model- or optimizer-owned state). Moved here: the training loop hands the model's own grad tensors in between steps; calling it on unrelated tensors is inert and never touches SGD velocity or Adam m/v.
- `adam_keeps_independent_moment_state_per_parameter` / `sgd_momentum_velocity_is_independent_per_parameter`: deleted narration preambles — both restate the test name (the independence property is already pinned above).
- Kept inline: the arithmetic derivations for the expected values (`1.0 - 0.1*0.5`, the v=1.0/v=1.9 momentum steps, and Adam's t=1 update reducing to ~lr*sign(g)).
