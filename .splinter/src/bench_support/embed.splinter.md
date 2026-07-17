# src/bench_support/embed.rs — commentary

- `DIM` tuning history: each token deposits 4 signed values, so a ~10-token document writes ~40 slots. Into 64 dims that is ~40% collisions, which drowned the token-overlap signal and made the dense leg near-noise (mean recall@10 0.45 on synthetic.json). At 512 collisions drop below ~8%, so cosine faithfully tracks token overlap and the harness reaches recall@10 1.0 with no residual hashing artifact. Still bench-only; never a substitute for a real semantic model.

Second-pass migration (from the `//!` doc):
- Mechanism: maps text to a fixed-`DIM` vector by feature-hashing each token into signed slots (4 signed deposits per token), then L2-normalizing. There is no learned model.
- Consumers: the bench harness (`build.rs`, `replay.rs`) uses it to exercise the retrieval/index path at scale without a live Ollama embedder.
- The "never wire into production" rule and the token-overlap-not-meaning trap stay inline; the rest of the old 8-line module doc lives here.
# src/bench_support/embed.rs — commentary (migrated from source doc comments)

- This is a deterministic feature-hashing embedding STUB for benchmarks only. It is NOT a semantic embedder: cosine reflects token OVERLAP, not meaning. Never wire it into production. (A tightened one-line warning is kept inline at the top.)
- `DIM = 512`, chosen not smaller: fewer dimensions collide enough to drown the token-overlap signal.
- Empty / token-less input → norm 0 → `l2_normalize` leaves the vector all zeros (no NaN). Guarded by `empty_or_tokenless_input_is_a_zero_vector`.
