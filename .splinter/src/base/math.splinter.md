# src/base/math.rs — commentary

- `l2_normalize`: placed here beside the other `base::math` primitives deliberately, so every retrieval/fusion path shares one implementation instead of growing local copies.
- `OnlineSoftmax::finalize`: the corroboration boost matters in practice for entities surfaced via multiple retrieval paths, e.g. both the seed list and the beam in `retrieval::merge`.
Second-pass migration (2026-07-17):
- `OnlineSoftmax::finalize` doc trimmed to the pooling contract; full derivation here: a single observation is the identity (`x + ln 1 = x`); k observations at score x finalize to `x + ln k` — e.g. twice at 0.8 ≈ 1.49, outranking a single 0.9 (pinned by `corroborated_item_can_outrank_higher_single_observation`). The result is a relevance magnitude, not a probability — values above 1.0 are expected and fine; downstream only ranks by it and applies a multiplicative confidence plus additive boosts. Use `running_max` when best-score-wins (no corroboration) is wanted.
- `avx2_path_matches_scalar_reference` doc trimmed: on an AVX2+FMA machine the public `cosine` takes the SIMD path; the test asserts it agrees with the scalar reference.
