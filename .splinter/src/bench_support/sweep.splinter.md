# splinter: src/bench_support/sweep.rs


Second-pass migration (from the `SweepParam` variant docs):
- `RrfK`: reciprocal-rank-fusion constant `k` in `1/(k + rank)`. A larger `k` flattens the rank-weight curve so later ranks contribute relatively more. The typical 10–60 range stays inline.
- `MinDeliverScore`: the minimum blended score a hit must clear to be delivered. Higher trims the tail — precision over recall. The `[0.0, 1.0]` range stays inline.
- `MmrLambda` (relevance-vs-diversity endpoints) and `SeedK` (integer `>= 1`; a swept value `< 1` is clamped to 1 with a warning since 0 seeds nothing) keep their docs inline — both are contracts, and the SeedK clamp is a trap guarded by `apply_clamps_seed_k_below_one_to_one`.

Strict-bar pass (comments): all four `SweepParam` variant doc comments were removed from source (previously inline). Semantics preserved here:
- `RrfK`: RRF constant `k` in `1/(k + rank)`, typically 10–60.
- `MinDeliverScore`: minimum blended score in `[0.0, 1.0]` a hit must clear to be delivered.
- `MmrLambda`: MMR relevance-vs-diversity tradeoff in `[0.0, 1.0]` — 1.0 = pure relevance, 0.0 = pure diversity.
- `SeedK`: number of seed entities pulled before graph expansion; integer `>= 1`, a swept value `< 1` is clamped to 1 (with a warning) since 0 seeds nothing.
