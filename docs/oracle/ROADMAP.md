# Roadmap — the single source of truth

State and work, one file. `FEATURES.md` says what exists, `CHANGELOG.md` says
what was decided, `VISION.md` says what "built" means. This file is the only
place that says **what is left**. Nothing else in the repo plans work.

Stamped 2026-07-20, HEAD `2878223`. Verified against source, not against docs.

---

## 1. North star

kern equals or beats Zep/Mem0-class agent memory on LoCoMo / LongMemEval while
staying local-first, in-process, per-cwd, offline-capable, no query-time LLM on
the default path.

**Recorded baseline** (`docs/kern/locomo-baseline-2026-07-19.json`, full
locomo10, 1986 QA, seeds 0/1/2, `qwen3-embedding:0.6b` + `granite4:3b` +
`qwen2.5:7b` judge):

| category | n | F1 | judge+abstain |
|---|--:|---|---|
| single-hop | 282 | 0.104 ± 0.004 | 0.093 ± 0.005 |
| multi-hop | 321 | 0.023 ± 0.003 | **0.042 ± 0.011** |
| temporal | 96 | 0.118 ± 0.013 | 0.194 ± 0.016 |
| open-domain | 841 | 0.118 ± 0.006 | 0.194 ± 0.013 |
| adversarial | 446 | 0.0 | **0.112 ± 0.103** |
| **overall** | 1986 | — | **0.137 ± 0.018** |

Latency p50 901 ms / p95 1839 / p99 2666. Published rivals sit ~0.6+. **Gap:
0.46.** Two craters carry it: multi-hop and adversarial abstention.

Claim standard: no SOTA/parity/latency claim without a multi-seed run with error
bars against this file. Published-leaderboard gaps under ~10 points are noise —
LoCoMo's answer key is ~6% wrong and LLM judges are lenient.

---

## 2. How we supersede Zep / Mem0 / Letta / Qdrant

Not by matching feature lists. By owning a combination none of them hold, then
proving it on the eval.

| property | kern | Zep/Graphiti | Mem0 | Letta | Qdrant |
|---|---|---|---|---|---|
| Per-project self-maintaining graph (per-cwd) | ✅ | ❌ hosted | ❌ | ❌ | ❌ |
| Default recall touches no LLM (sub-ms) | ✅ | ❌ | ❌ | ❌ | n/a |
| Local-first, single binary, no network hop | ✅ | ❌ | ❌ | partial | ❌ |
| Self-forgetting (decay / stigmergy GC / cold spill) | ✅ | ❌ | partial | ❌ | ❌ |
| Graph + dense ANN + BM25 + GNN in one process | ✅ | partial | ❌ | ❌ | ❌ |
| Bi-temporal supersede off the recall path | ✅ | ✅ | ❌ | ❌ | ❌ |
| Coordinator-free CRDT federation | 🟡 building | ❌ | ❌ | ❌ | ❌ |
| Published eval numbers | 🟡 0.137 | ✅ | ✅ | ✅ | n/a |

**The three moves, in order:**

1. **Close the eval gap.** The architecture argument is already won on paper and
   lost on the scoreboard. Nothing else matters until overall clears ~0.5.
   Everything in §3 serves this.
2. **Ship what a hosted service structurally cannot.** Offline, per-cwd, zero
   egress, sub-ms default recall, self-forgetting. These are not features they
   are behind on — they are features their business model forbids. Federation
   (§5) is the same bet: no shipped competitor has it.
3. **Refuse the vector-DB fight.** Qdrant parity (PQ, payload indexes, sharding,
   RBAC, SDKs, multitenancy) is a non-goal — see §8. Mounting Qdrant as a
   backend yields a superset, not supersession, and forfeits the only structural
   advantage kern has. Repo law forbids a pluggable backend.

Closest rivals per axis: **YourMemory** (decay + published LoCoMo, claims +16pp
over Mem0 — read before quoting ourselves), **Graphiti** (temporal semantics),
**mnemo** / **AgentDB** (Rust + embedded + MCP stack), **Cognee** (self-hosted
KG). Full survey: `docs/kern/`.

---

## 3. Eval work — everything blocks on the ablation suite

Instruments landed 2026-07-20; the runs have not happened. Sequence is strict.

- [ ] **A. Run the attribution ablations.** `--context-mode kern | grounded |
      grounded-retrieval` at full scale. Grounded needs 32k ctx (conversations
      measure 11–24k tokens; 8k/16k truncate silently). Deliverable: a signed
      attribution table splitting the 0.46 gap into retrieval loss vs synthesis
      loss vs distill loss. **This gates B, C, D, and half of §6.**
- [ ] **B. Land abstention.** Prompt + empty-context short-circuit shipped and
      unit-pinned. Remaining: the `--min-deliver 0 / 0.2 / 0.4` floor sweep and
      the seed-0 re-run. Target: adversarial ≥ 0.5, no regression elsewhere.
      (`MIN_DELIVER_SCORE` was dead code — shipped default 0.0 never gated
      delivery. Deleted.)
- [ ] **C. Close the multi-hop crater (0.042).** The "expansion is one hop"
      hypothesis is **dead** — `expand()` is a beam search. Live bounds:
      `max_expansions=500`, the `score < global_best*0.25` prune, and whether
      the edges exist at all. Smoke n=8: 8/8 had nearby claims, only 4/8 linked
      within 2 hops → **ingest-side edge creation is the prime suspect**. Run
      `--multihop-paths` at full scale, then fix the side it names.
- [ ] **D. Measure distill coverage.** Rides A's grounded-retrieval run:
      `gold_nearest_cosine` p10/p50/p90 + share ≥0.6. Smoke n=6: p50 0.464, only
      1/6 over 0.6 — but calibrate the 0.6 bar first (2–4-word golds vs sentence
      claims).
- [ ] **E. Calibrate the judge.** Untouched. 50 hand-labeled verdicts, agreement
      ≥0.9. Until then no category delta under 5 points is real.
- [x] Baseline recorded, 3 seeds, Wilson CIs + exact McNemar paired A/B
      (`--compare-probes`), per-phase wall clock, `--concurrency 4`.
- [x] Temporal: distill resolves relative dates against the session header
      (**eval side only — the product intake still has the gap**).
- [x] Answer shape: `QueryOptions::answer_style` (**eval-only by design**).

---

## 4. Retrieval & latency

- [ ] **Gate HyDE off on strong lexical/cache hits.** Costs an LLM call when the
      cheap path already won. Judge against baseline.
- [ ] **Move `merge_hits` onto RRF.** Raw `0.4·content + 0.6·GNN` blend is
      fragile across scales; `fuse::rrf` already fuses the answer layer.
- [ ] **O(N) importance scan per retrieve** (`retrieval/seed.rs`) — the scaling
      cliff at query time. Top structural debt in the whole repo.
- [ ] **Speculative decode** (qwen3.5:0.8b draft → 4b generator) — the last open
      lever on answer latency. Streaming, capped `num_ctx`, warm-keeping shipped.
- [ ] Min-max normalize scoring in `apply_boosts`; swap the hand-rolled stemmer
      for `rust-stemmers` 1.2.0 + stopwords (needs a BM25 rebuild);
      validate-or-remove GNN reranking. All three are measurement-gated on §3A.
- [ ] Query cache keys on vector hash only — near-identical queries miss.
- [ ] HNSW tombstone compaction — dead nodes accumulate.
- [ ] No learned rerank model — every rerank is a cold LLM call.
- [x] Filtered ANN end-to-end (all three seed sources on `is_active`, recall@10
      A/B `9386de0`); RRF at the answer layer; `answer:false` sub-ms no-LLM path;
      semantic query cache (cosine ≥0.97 + version stamp); lock-scoped answer
      path; workload regression trace + sweep.

---

## 5. Federation (`building`, off by default)

Phase 1 landed inline — lamport-stamped LWW on `Reason.score` and `valid_until`
(`base/merge.rs`), `union_statements` OR-Set semantics, `PendingDelta` queue and
`start_delta_flush` Delta sender. `crdt.rs` is still 90 LoC of `GCounter` only;
the LWW/OR-Set semantics live as inline fields, not as named types. Fine.

Missing, verified by grep at HEAD:

- [ ] **Pulse and Question senders.** Handlers exist, no emitter. `handle_question`
      is dead code; clustering does not federate.
- [ ] **Anti-entropy.** No `AntiEntropy` wire variant. `Fetch` is single-id only,
      and `set_fetch_handler` is never called, so every reply is `found:false` —
      the fetch RPC is unwired. `EntitySync` ships only the hottest 32 per
      heartbeat, so cold entities may never propagate. A partitioned node that
      rejoins never catches up.
- [ ] **Transport security.** Raw TCP, no TLS. `network_id` broadcast cleartext
      over UDP multicast. No signature on `GossipMessage`; `handle_conn` accepts
      any stream and `handle_peer_exchange` trusts any `msg.origin`. Needs
      `tokio-rustls` + `rcgen` as direct deps. **This one gates any deployment
      off a trusted LAN / WireGuard mesh.**
- [ ] **Backpressure.** No per-peer rate limit, no divergence metric in
      `HealthStats`, remote heat is an unclamped `f64` joined by max → pinnable.

Four decisions owed before the build (deciding behavior: **none yet — amend
first**):

- (a) Does `Reason.score` stay LWW, or revert to max-join for monotone trust
      signaling? (`degrade_entity_reasons` lowers it, so max-join silently
      reverts deliberate degrades — LWW looks right, but it is not recorded.)
- (b) Anti-entropy watermark shape: vector clock or content-hash bloom?
- (c) TLS cert authority: operator PKI or TOFU pin?
- (d) Does `network_id` derive from the cert or stay config-owned?
- (e) Does graviton `mass` federate at all, or stay per-node tuning? Two peers
      can currently disagree on a graviton's pull.

Wire-format impact: `GossipMessage.signature` is breaking (mitigate with
`serde(default)`); `AntiEntropy` is additive. Confidence isolation
(`conf_alpha`/`conf_beta`/`unlinked_count` never imported from remote) must
survive every change.

---

## 5x. Hub — machine-level control plane (`active`, phases 1-2 + merge landed)

One binary, two roles. Landed 2026-07-20: `kern hub` supervisor —
resolve/spawn/adopt/unload nodes, hub-first `kern mcp`, graceful shutdown via
`KernRpc::shutdown`. Data path stays client→node direct.

- [x] **Phase 2: idle lifecycle.** `HealthRes.idle_ms` (last real tool call,
      health polls excluded; stamped in the MCP dispatch core AND every typed
      RPC method); hub reaper double-checks under the root lock, then
      gracefully unloads. `--idle-unload-secs`, default 1800, 0 disables.
      Hub-owned nodes only — hand-started daemons are the user's to stop.
- [ ] **Phase 3: gossip moves hub-side.** One UDP endpoint + one node identity
      per machine; nodes stop binding the network entirely. Collapses the
      per-project port-clash validation in `config/serve.rs`. **Ordering
      decided 2026-07-20:** §5 senders and semantics (a-e) build per-node
      first; this transport move ships together with §5's TLS work — same wire
      layer, migrate once. Not blocked, sequenced.
- [x] **Phase 4 (merge half): `kern hub merge <src> <dst>`.** Offline CRDT
      union via `absorb_graph`: both daemons stopped first, src never written.
      Cross-kern navigation beyond resolve/status remains future work.
- [x] Hub auto-start: `kern mcp` spawns a detached hub when none answers
      (`[hub] auto_start = false` opts out); `kern hub stop` ends it over RPC.

---

## 6. Correctness, safety, product gaps

Ordered by leverage.

- [ ] **Validate `Kind` at the wire boundary.** A caller can claim `Fact` or
      `Superseded` today. Highest-leverage safety fix in the repo — `Fact` is
      GC-immune, so a forged one is permanent.
- [ ] **Enforce `valid_until` in retrieval.** The field is dead code; expired
      claims still rank.
- [ ] **Source-keyed idempotency at ingest** — `(source.system, object_id,
      section)` instead of paraphrase-evadable cosine dedup.
- [ ] Clamp `conf` to [0,1]. Require reason text on supersede.
### Getting data in — the intended shape, and what breaks it

Three ways in, and they are not equals. **MCP `ingest` and `kern ingest` are the
main path.** The **intake** (`.kern/intake/`) is the drop directory: put a
document or a text file in it and it lands in the graph, no call required.
Anything that costs a user friction on either path is a bug, not a feature
request. One name for all of it — `intake`, everywhere, no second word.

- [x] **The intake accepts anything readable as text.** Was `.txt`-only with a
      silent early return; now routes by what the file is — `.txt` is a
      transcript and is distilled, everything else is a `Document` stored whole
      via the watcher's path, binary is quarantined into `failed/`. Documents
      therefore need no reason LLM, so the drain always runs.
- [x] **One vocabulary.** The old name is gone tree-wide: `IntakeConfig`,
      `[intake]`, `.kern/intake/`, `spawn_intake`, `kern.intake`. The legacy
      directory migrates itself on first start when the new path is free.
- [ ] **No way to see or drive the intake.** Nothing reports what is pending,
      what failed, or why; failures surface only as tracing warnings inside the
      daemon. Wanted: `kern intake` (list pending + failed with the last error)
      and a one-shot drain so the CLI works without a running daemon.
- [ ] Intake distillation still lacks the relative-date resolution the eval path
      got (§3) — dropped text with "last Tuesday" in it stores unresolved.

- [ ] **Automatic session intake has no producer.** The transcript lane is
      complete and tested end-to-end, but its writer was a Claude Code Stop hook
      deleted in `483b37c`, and the plugin was removed when kern was reframed as
      agent-agnostic. Prompt-time recall is gone; recall is now query-only.
      Until a producer exists, `VISION.md`'s "intake is a byproduct of working"
      is false. Restoring agent-specific hooks would undo the agent-agnostic
      decision — so the answer is the intake itself: **document the drop-a-file
      contract and let any harness write into it.** That makes the fixes above
      the prerequisite, not a side quest.
- [ ] **CLI vs daemon race** — CLI reads the on-disk graph while the daemon
      holds newer state. Needs `kern status` + advisory locking.
- [ ] Per-kern entity cap is `KERN_CAP_DISABLED` and marked unsafe to enable.
- [ ] GNN training runs synchronously on the tick — stalls large kerns.
- [ ] Config section-level replace is not a deep merge: a project section
      silently drops keys the user section set.
- [ ] `validate_fact_source` is dead code (sole caller passes the literal
      `AGENT_SOURCE`). Decision: thread a real auth identity, or delete. Delete
      is correct for a single local daemon and needs only sign-off.
- [ ] Distill prompt is one-shot and global — no per-descriptor prompts, no
      long-delta chunking. `kind` label accuracy ~33% even at 7B; the taxonomy
      has overlapping categories (decision/project, fact/code-fact).
- [ ] Two parallel typed transport surfaces (`kern_rpc` + `search`) with
      overlapping DTOs — one should die.
- [ ] Document gravitons truncate at the embed context window; chunk+mean-pool
      is the upgrade path. Blocked on a real document long enough to truncate.
- [x] Durability: `snapshot_if_dirty` on the maintenance tick. WAL rejected —
      LMDB already orders recovery.

Deferred design calls, still owed, no blocker but no urgency: quarantine
representation (bool vs `EntityStatus::Quarantined` vs `Source` trust band);
contradiction-reconcile gating band; temporal-aware as-of retrieval scoring;
episodic abstraction as a tick task; chunking strategy (contextual-prepend vs
proposition self-containment). Threat model and the staged hardening list
(ed25519 signing, peer trust, Sybil binding, replay protection, ACL enforcement)
lived in `docs/kern/safety-architecture.md` (deleted 2026-07-20, stale paths; recover
from git history if the hardening track opens).

---

## 7. Serving other agentic systems (the embeddable-endpoint track)

kern's competitive claim in §2 is "everything a hosted service structurally
cannot do". The flip side is that a hosted service can serve *many callers* and
kern currently assumes exactly one. Every item here is what a host system —
Alois was the driving case — needs before it can mount kern as its reasoning
store instead of Zep or a vector DB. Audited against source at v1.0.0
(`85c4fef`); the change points named below were all verified to exist.

This is the second-most-valuable track after §3, because it converts kern from
"my agent's memory" into "the memory layer any agentic workflow embeds".

### 7a. ACL + request principal — gates everything else here

- [ ] **Expose ACL on `ingest`.** `Entity` already carries `Acl` (serde'd,
      empty = public), but the MCP `ingest` schema has no `principals`/`scope`
      property, so nothing can ever populate it. Add both, thread through
      `ingest::Job` into `place.rs`.
- [ ] **Accept `principals` on `query`.** No identity param exists today
      (`tools_query.rs`). Add to schema + `QueryOptions`.
- [ ] **Enforce at retrieval.** `matches_filter()` (`retrieval/score.rs`) drops
      entities sharing no principal with the requester. **The id-path in
      `tools_query.rs` bypasses every filter via a direct `find_entity` lookup
      — it needs the same guard or ACL is decorative.**
- [ ] Decide: does the file watcher give `Document` entities a tenant-default
      ACL, or leave them public? Recommend configurable, default
      public-within-tenant, since the tenant boundary is the process.

Two constraints that must hold: **ACL is caller-asserted** — the daemon cannot
verify a caller's principals, exactly like the existing `validate_fact_source`
boundary, so trust ends at the process edge. And **Facts are GC-immune, not
ACL-immune** — a Fact the requester can't see still must not be returned.
Backward compatibility: empty `principals` means *no filter*, not *public
only*, or every existing single-agent caller goes blind.

### 7b. Review / draft lifecycle

- [ ] `ReviewState` on `Entity` (`#[serde(default)]` → old rows decode as
      `PendingReview`, fail-safe) + source-level review policy in config +
      an `exclude_pending` query filter and a `promote` tool. Lets a host hold
      auto-distilled claims out of retrieval until a human curates them.
      Requires 7a's `QueryOptions` work first — review filters are just more
      `matches_filter` predicates.

### 7c. Source-trust weighting

- [ ] User-authored claims should outrank auto-ingested claims of equal heat.
      `apply_boosts` has no source-trust prior today. Add
      `source_trust_user`/`_agent`/`_auto` to `RetrievalConfig` (default all
      `1.0` so ranking does not move until configured) and multiply in the
      boost step — **post-fusion, not in RRF**, which is rank-based.
      Independent of 7b; can run parallel after 7a.

### 7d. Retention / right-to-be-forgotten

- [ ] **`forget_by_source(scheme, object_id)`** — deleting a source in the host
      must cascade into the graph. `forget` exists but is per-entity and Facts
      are immune. Needs a `force` param that punches through the Fact guard:
      a legal deletion outranks GC-immunity. **This is the only place that
      guard may be bypassed, and it must be explicit, never default.**
- [ ] Per-source TTL — an ingest-time `retention` duration setting
      `valid_until`. Nearly free: one param plus one timestamp, and the
      existing bi-temporal expiry path already enforces it *once §6's
      "enforce `valid_until` in retrieval" lands*. Blocked on that.

### 7e. Deferred

- In-kern token metering. Gateway-side metering (the host proxies model
  endpoints and counts per tenant) needs zero kern change and works today.
  Revisit only if background distillation becomes a real cost surprise; the
  intake's retry-until-success behaviour (`ingest/intake.rs` — `finalize`
  archives only on full success) already models the "pause work" signal a
  budget ceiling would need.

---

## 8. Non-goals

None of these move an agent-memory eval score. All are table stakes only in a
multi-tenant hosted-DB business kern is not in. Revisit only if that business
materializes.

Distributed sharding (Raft) · replication + write-consistency factor · API key /
JWT-RBAC / TLS-for-clients / audit logging · public REST + gRPC + multi-language
SDKs · multitenancy · GPU index building · product quantization / SPLADE sparse
vectors / ColBERT multi-vector — re-promote any one of these **iff** §3A shows a
retrieval-quality gap it would close.

**Parked indefinitely:** the v2 self-training track (LoRA in Rust, teacher
pipeline over mature graph regions, per-graviton adapters hot-swapped at query
time, adapters gossiped by content hash). Nothing built, nothing scheduled.
Gate: an overall eval score that makes specialization worth funding.

---

## 9. Repo laws

1. **Append-only bincode.** Persisted enums/structs grow by appending only;
   guard schema touches with a round-trip test.
2. **No pluggable/fallback backend.** All-internal, in-process, self-contained.
3. **One dispatch core.** Every surface goes through `tools::dispatch`, never a
   second copy.
4. **This file is the only plan.** New work goes here, not into a new document.
