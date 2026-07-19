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
4. **What must federation prove before it earns senders?** The code audit is
   done (`docs/federation-integration-plan.md`): every roadmap claim verified
   against source — Delta/Question/Pulse have handlers but no senders, no
   `AntiEntropy` wire variant, `crdt.rs` has only GCounter/PnCounter,
   `statements` is never merged, `Reason.score` max-join is the wrong rule for
   a non-monotonic field (degrade lowers, max-join loses it), transport is
   raw TCP with cleartext UDP `network_id`. Four decisions gate the build:
   (a) does `Reason.score` move to LWW-Register, or keep max-join for
   monotonic trust signaling? (b) anti-entropy watermark shape — vector clock
   or seen-set snapshot? (c) TLS cert authority — operator PKI or TOFU? (d)
   does `network_id` derive from the cert or stay config-owned? Blocker: none —
   the plan is additive to the existing wire enum and merge path. Deciding
   behavior: none yet — amend first.
5. **When does the LLM answer path get interactive?** Streaming, capped
   context, and warm-keeping shipped; speculative decode (draft → generator) is
   the open lever. Blocker: the baseline (1) for a before/after number.
   Deciding behavior: verify-before-claiming.
6. **Does `src/wire.rs` survive?** 36 DTO structs, 3 validate fns actually
   imported (`tools_mutate.rs`); the live RPC DTOs are
   `src/trnsprt/src/kern_rpc/dto.rs`. Question: delete the dead 33 and move the
   validators next to their one caller, or is wire.rs a planned external API
   surface? Deciding behavior: delete-superseded.
7. **Does graviton `mass` federate?** Kern-shell fields (graviton, radii, mass)
   stay local under the current CRDT merge; two peers can disagree on a
   graviton's pull. Question: is mass per-node tuning (stay local) or shared
   graph shape (needs a sender + merge rule, folds into 4)? Deciding behavior:
   none yet — amend first.
8. **When do document gravitons outgrow one embed call?** Long seed documents
   truncate at the embed model's context window (`ponytail:` ceiling in
   `tools_admin.rs`); chunk+mean-pool is the upgrade path. Blocker: a real
   document long enough to truncate. Deciding behavior: verify-before-claiming.
