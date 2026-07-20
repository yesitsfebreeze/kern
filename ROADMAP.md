# Roadmap

Decisions ahead, ordered. Questions, not tasks.

1. **What closes the multi-hop crater?** The recorded baseline
   (`docs/kern/locomo-baseline-2026-07-19.json`, overall 0.137 ± 0.018)
   puts multi-hop at 0.042 ± 0.011 — reason-edge expansion is not
   connecting facts across sessions. Question: is the failure in distill
   (claims land unlinked), expansion depth/scoring, or fusion drowning
   expanded hits? The deciding instruments landed 2026-07-20
   (`--context-mode` ablations, `--multihop-paths` connectivity
   diagnostic); note `expand()` is a beam search, not one-hop — the
   one-hop hypothesis is dead, the missing-edges one is live. Blocker:
   none — run the diagnostics. Deciding behavior: verify-before-claiming.
2. **Where does abstention come from?** Adversarial abstain was
   0.112 ± 0.103 — near-unseeded. As of 2026-07-20 the answer prompt
   instructs declining with the exact marker string and empty-context
   synthesis short-circuits to it; what remains is the measurement:
   adversarial ≥ 0.5 with no regression elsewhere. Blocker: the seed-0
   re-run. Deciding behavior: verify-before-claiming.
3. **When does HyDE run?** Query expansion costs an LLM call even when the
   cheap lexical/cache path already wins. Blocker: none — gate against the
   recorded baseline. Deciding behavior: verify-before-claiming.
4. **Does the dense seed merge move onto RRF?** `merge_hits` blends raw scores
   (0.4 content / 0.6 GNN), fragile across scales, while `fuse::rrf` already
   fuses the answer layer. Blocker: none — judge against the recorded
   baseline. Deciding behaviors: verify-before-claiming, delete-superseded.
5. **What must federation prove before it earns senders?** The code audit is
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
6. **When does the LLM answer path get interactive?** Streaming, capped
   context, and warm-keeping shipped; speculative decode (draft → generator) is
   the open lever. Blocker: the recorded baseline for a before/after number.
   Deciding behavior: verify-before-claiming.
7. **Does `src/wire.rs` survive?** 36 DTO structs, 3 validate fns actually
   imported (`tools_mutate.rs`); the live RPC DTOs are
   `src/trnsprt/src/kern_rpc/dto.rs`. Question: delete the dead 33 and move the
   validators next to their one caller, or is wire.rs a planned external API
   surface? Deciding behavior: delete-superseded.
8. **Does graviton `mass` federate?** Kern-shell fields (graviton, radii, mass)
   stay local under the current CRDT merge; two peers can disagree on a
   graviton's pull. Question: is mass per-node tuning (stay local) or shared
   graph shape (needs a sender + merge rule, folds into 5)? Deciding behavior:
   none yet — amend first.
9. **When do document gravitons outgrow one embed call?** Long seed documents
   truncate at the embed model's context window (`ponytail:` ceiling in
   `tools_admin.rs`); chunk+mean-pool is the upgrade path. Blocker: a real
   document long enough to truncate. Deciding behavior: verify-before-claiming.
