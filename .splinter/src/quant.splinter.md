# src/quant.rs — commentary

- `QuantizedVec.dim_bits`: adding this serde field was safe because `QuantizedVec` is in-memory only and never persisted — the on-disk projection is `store::StoredVec`.
- `QuantizationMode::parse`: the reason Binary is not config-exposed (recall ~0.33 without rescore) stays inline at the parse arm; the measurement lives in `hnsw::tests::binary_recall_tracks_f64`.
- `bytes_per_dim` returns `f32` deliberately — it feeds display/back-of-envelope math and keeping it narrow avoids silent widening at printf-style call sites.
Second-pass migration (stricter bar):
- module: int8 = 1 signed byte/dim (4x smaller), binary = 1 sign bit/dim packed 8/byte (~32x smaller), Hamming-ranked for candidate gen, rescored with retained f32.
- `QuantizationMode::parse`: Binary recall@10 measured ~0.33 vs int8 0.75 (`binary_recall_tracks_f64`); wired + tested internally via `with_mode`, excluded from config until int8-rescore lifts the floor.
- `bytes_per_dim`: narrow f32 deliberately — feeds display/back-of-envelope math, avoids silent widening at printf-style call sites.
- `decode` Binary arm: reconstructs ±1.0 per sign bit; coarse by design, only a fallback (the `_` arm of `quantized_cosine_distance`) since search rescores with f32.
- `binary_cosine_distance`: sign-random vectors disagree per-dim with probability θ/π, so θ ≈ π·hamming/dim, distance = 1 − cos(θ).
- `float_cosine_distance`: zero-norm input gives similarity 0.0 from `base::math::cosine`, hence distance 1.0.
- `mixed_mode_exactly_matches_the_decoded_float_distance`: the `< 1e-2` sibling test only proves "small" (int8 lossy, same-content mixed pair never < eps); exact contract is fallback decodes BOTH operands and delegates to `float_cosine_distance`, order-symmetric.
- `binary_packs_one_sign_bit_per_dim`: byte0 bits {0,2,4,6} = 0x55, byte1 bit0 = dim 8.
- AVX2-vs-scalar test lengths span the 16-wide chunk boundary and tail (0,1,7,15,16,17,31,33,64,100).
## Design context (moved from source doc comments)

- Module: vector quantisation for the SEARCH INDEX (int8 / 1-bit sign); the original f32 is kept for rescoring. This is NOT LLM-model quantisation.
- `QuantizationMode::bytes_per_dim`: size estimates only.
- `QuantizedVec.b`: packed sign bits for `Binary` mode (8 dims/byte), empty otherwise.
- `encode_binary`: one sign bit per dim, `1` iff `x >= 0.0` — zero counts as positive.
- `binary_cosine_distance`: Hamming-based cosine-distance estimate in `[0, 2]` (same scale as the float/int8 paths), monotone in Hamming distance.
- Test `int8_avx2_dot_norms_match_scalar_reference`: -128 is included in the fixtures although `encode` never emits it — the AVX2 kernel must still match scalar on it. Uses a deterministic LCG so the test needs no rng dependency.
- Kept in source (load-bearing): `Binary` variant is in-memory only (on-disk `StoredVec` stays int8); `parse` deliberately rejects "binary" (recall floor too low without rescore, see `binary_recall_tracks_f64`); `dim_bits` is the true Binary dimension because the padded last byte makes `b.len()*8` over-count; the AVX2 SAFETY INVARIANT block on `int8_dot_norms_avx2`.
