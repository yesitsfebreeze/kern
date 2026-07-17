# Roadmap

Decisions ahead, ordered. Questions, not tasks.

1. **What is the recorded eval baseline?** Every retrieval/distill change is
   currently judged against intuition. The `locomo_eval` harness is validated
   end-to-end and the GPU blocker is root-caused and fixed (kern's own
   `num_gpu:0` serving pin leaking into eval via the native-path routing;
   `docs/kern/eval-locomo.md`) — measured p50 query latency is 2.3 s on GPU.
   Blocker: none — the multi-sample, multi-seed run itself remains to be run
   and recorded. Deciding behavior: verify-before-claiming.
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
5. **What must federation prove before it earns senders?** Delta/Question/Pulse
   and the fetch RPC are handled on receipt but never sent; transport is
   unauthenticated and unencrypted; batch size, push vs. pull, and anti-entropy
   are untuned. Blocker: a security stance for untrusted segments
   (`docs/FEDERATION-SECURITY.md`) — none of the pinned behaviors decides it.
   Deciding behavior: none yet — amend first.
6. **When does the LLM answer path get interactive?** Streaming, capped
   context, and warm-keeping shipped; speculative decode (draft → generator) is
   the open lever. Blocker: the baseline (1) for a before/after number.
   Deciding behavior: verify-before-claiming.
