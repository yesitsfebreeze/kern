Second-pass migration:
- `Model` contract doc compressed to the two ordering rules. Full contract moved here:
  - `forward` runs layers in order; when `residual` is set and a layer preserves its input shape, the input is added back to the output (dims checked before the add, so the `expect` never fires). `out_layer` applies last if present. Each layer caches state for its backward pass.
  - `backward` walks layers in reverse, mirroring the residual add on the incoming gradient.
  - `parameters`/`param_grads` (and `_mut` forms) expose trainable tensors and grads in matching order for an optimizer to step.
- `residual_model_adds_the_input_back` comment compressed; oracle: rebuild the bare layer with the same RNG seed, then residual output must equal layer_out + input element-wise (< 1e-9).