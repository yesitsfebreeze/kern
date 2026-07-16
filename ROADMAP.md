# Roadmap

Decisions ahead, ordered. Questions, not tasks.

1. **What is the recorded eval baseline?** Every retrieval/distill change is
   currently judged against intuition. The `locomo_eval` harness was validated
   end-to-end on the default local models (1 sample / 3 QA, `docs/kern/eval-locomo.md`):
   pipeline runs, report shape correct. Blocker: a real multi-sample,
   multi-seed baseline needs the host chat models on GPU — they currently run
   on CPU (~50 s per one-token call, ~11–27 h extrapolated for the full run),
   so any number recorded now would measure CPU-bound generation, not the
   configured models. Deciding behavior: verify-before-claiming.
2. **When does HyDE run?** Query expansion costs an LLM call even when the
   cheap lexical/cache path already wins. Blocker: the baseline (1) — gating
   must show a measured win, not a plausible one. Deciding behavior:
   verify-before-claiming.
3. **Does the dense seed merge move onto RRF?** `merge_hits` blends raw scores
   (0.4 content / 0.6 GNN), fragile across scales, while `fuse::rrf` already
   fuses the answer layer. Blocker: the baseline (1). Deciding behaviors:
   verify-before-claiming, delete-superseded.
4. **Which durability primitive lands first — snapshots or WAL?** A memory that
   loses what it was told is not a memory; both reuse the existing persistence
   path, neither exists. Blocker: none — ordering is the decision. Deciding
   behavior: name-the-tradeoff.
5. **Does HNSW insert become id-stable?** Blocks 10k-scale determinism and
   bench-build speed (`graph.rs`). Blocker: none. Deciding behavior:
   verify-before-claiming (determinism is a measured property).
6. **What must federation prove before it earns senders?** Delta/Question/Pulse
   and the fetch RPC are handled on receipt but never sent; transport is
   unauthenticated and unencrypted; batch size, push vs. pull, and anti-entropy
   are untuned. Blocker: a security stance for untrusted segments
   (`docs/FEDERATION-SECURITY.md`) — none of the pinned behaviors decides it.
   Deciding behavior: none yet — amend first.
7. **When does the LLM answer path get interactive?** Streaming, capped
   context, and warm-keeping shipped; speculative decode (draft → generator) is
   the open lever. Blocker: the baseline (1) for a before/after number.
   Deciding behavior: verify-before-claiming.
