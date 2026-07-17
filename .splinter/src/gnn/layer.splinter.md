# splinter: src/gnn/layer.rs

Second-pass migration:
- `LinearLayer::with_rng` doc compressed to the determinism contract, matching `GCNLayer::with_rng`'s wording. Moved here: bias is zero-initialised and draws no RNG value, so the RNG stream position after construction depends only on `in_features * out_features`.
- `try_backward` doc compressed to 2 lines. Moved here: it also bubbles tensor shape errors as `GnnError::Tensor`, and `MissingForwardState` fires after a state reset, not only before the first `forward`.
- `zero_grads`: deleted "in place — keeps the allocations" (restates `fill(0.0)`; the `backward_accumulates_across_calls_until_zeroed` assert message already names the in-place contract).
- Deleted test narration: the `// 2 samples x 4 features` trailing label and the infallible-degrades-to-zero preamble — both duplicated their assert messages.
- Kept inline: the `weight` / `bias` shape labels, the dInput shape derivation in the error arm, and the `d_bias`/`d_weight` magic-number derivations (both = 2.0).
