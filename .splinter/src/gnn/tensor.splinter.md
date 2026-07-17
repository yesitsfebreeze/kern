# src/gnn/tensor.rs — commentary

- `MATMUL_PAR_THRESHOLD` (64): empirical breakpoint, not a hard limit. Below it the rayon per-task scheduling overhead outweighs the gain on the small matrices kern multiplies (per-kern GNN layers are tens of rows), so the serial triple-loop wins. Retune if layer widths grow substantially.


Second-pass migration:
- `rand_with` doc compressed: pass a seeded `rand::rngs::StdRng` (any `RngCore`) for reproducible weight init in tests; `Tensor::rand` draws from system entropy via `rand::rng()`.
- `fill` doc compressed to the allocation-reuse point.
- Kept: manual `Debug` rationale (2 lines), `MATMUL_PAR_THRESHOLD` one-liner (tuning rationale above), test magic-number comments.
