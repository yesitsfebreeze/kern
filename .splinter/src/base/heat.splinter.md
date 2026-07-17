# src/base/heat.rs — commentary

Second-pass migration (comment -> note):
- `HeatConfig` units: `deposit_access` / `deposit_traversal` are **dimensionless
  heat units**, not ratios or durations. `deposit` sums one onto the current heat
  (after decaying it), and the total then decays over `half_life_secs`.
- Why `deposit_traversal` (0.5) is lower than `deposit_access` (1.0): a traversal
  merely passing through an entity is weaker evidence of relevance than a direct
  read/retrieval of it, so it deposits less. Default half-life is one week.
- `decayed` math: exponential decay, `lambda = ln(2) / half_life_secs`, applied as
  `heat * exp(-lambda * dt)` in f64 and narrowed back to f32 on return.
- Degenerate-input contracts (each pinned by a test):
  - non-positive heat -> 0.0 via the leading guard.
  - `since: None` -> nothing to decay from, heat passes through untouched.
  - `since` in the FUTURE of `now` (clock skew) -> `duration_since` is Err, and
    the choice is to return heat unchanged rather than extrapolate growth.
  - `half_life_secs == 0` would divide by zero; `.max(1.0)` clamps it to an
    effective 1s half-life, so the result is finite (no NaN/inf) and just decays
    very fast.
- Tests use `HL = 100` (a 100-second half-life) purely so the expected arithmetic
  is readable: 8 -> ~4 after one half-life, ~2 after two.
