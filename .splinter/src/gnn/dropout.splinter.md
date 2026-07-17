# splinter: src/gnn/dropout.rs

Second-pass migration:
- Inverted-dropout comment compressed to 2 lines. Moved here: scaling survivors by 1/(1-p) at train time keeps the expected activation sum equal to the unscaled inference pass, which is exactly why `forward` applies NO scaling when `training == false` — the eval path returns `input.clone()` verbatim.
- `backward_without_forward_mask_is_identity`: deleted the `// no forward() called -> last_mask is None` trailing label (the test name says it).
- Contract worth knowing (not inline): `backward` with `last_mask == None` is identity, and `forward` clears `last_mask` on both the eval and `p == 0.0` paths — so a train->eval->backward sequence correctly stops masking gradients instead of reusing a stale mask.
