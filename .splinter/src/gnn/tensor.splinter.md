# src/gnn/tensor.rs — commentary

- `MATMUL_PAR_THRESHOLD` (64): empirical breakpoint, not a hard limit. Below it the rayon per-task scheduling overhead outweighs the gain on the small matrices kern multiplies (per-kern GNN layers are tens of rows), so the serial triple-loop wins. Retune if layer widths grow substantially.


Second-pass migration:
- `rand_with` doc compressed: pass a seeded `rand::rngs::StdRng` (any `RngCore`) for reproducible weight init in tests; `Tensor::rand` draws from system entropy via `rand::rng()`.
- `fill` doc compressed to the allocation-reuse point.
- Kept: manual `Debug` rationale (2 lines), `MATMUL_PAR_THRESHOLD` one-liner (tuning rationale above), test magic-number comments.

## Preserved from stripped comments (2026-07-17)
- `rand_with` uses a Box-Muller normal init with a caller-supplied RNG — seed it for deterministic tests; production `rand` uses system entropy.
- `MATMUL_PAR_THRESHOLD` (=64) is the row count at/above which `matmul` parallelizes across output rows with rayon; below it takes the serial path. Both paths are asserted numerically identical at the boundary (m = threshold and m = threshold-1).
- `fill` is in-place on purpose: it keeps the existing allocation, unlike assigning a fresh `zeros`.
- Manual `Debug` (not derived) prints shape + only an 8-element preview so logging a large weight tensor doesn't dump thousands of floats. (This note is kept in source too.)
