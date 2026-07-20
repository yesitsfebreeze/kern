# Roadmap — the single source of truth

State and work, one file. `FEATURES.md` says what exists, `CHANGELOG.md` says
what was decided, `VISION.md` says what "built" means. This file is the only
place that says **what is left**. Nothing else in the repo plans work.

Stamped 2026-07-20, re-verified against source after `8d8b19e`, `6c4a97f`,
`d992432` and the docs audit. Verified against source, not against docs.

---

## 1. North star

kern is the memory layer an agent recalls from: local-first, in-process,
per-cwd, offline-capable, self-forgetting, with no query-time LLM on the
default path — and it **retrieves the right thing**, provably.

**There is no recorded baseline.** The LoCoMo eval, the retrieval bench, and
`docs/kern/locomo-baseline-2026-07-19.json` were all deleted in `8d8b19e`
(2026-07-20). That deletion was correct and is not to be undone as-was: the
LoCoMo score collapsed ingest × retrieval × answering into one LLM-judged
number in which the **answering term dominated**. Measured the same day, a
grounded run — whole conversation in the prompt, kern bypassed entirely —
scored 0.187 on a slice where kern scored 0.027. The ceiling was set by a 3B
answerer, not by memory, so the number could not steer memory work. Three
eval-side prompt changes moving one slice from 0.131 to 0.027 in a single day
confirmed it was measuring the harness.

The previously published figures (overall 0.137 ± 0.018, "gap 0.46") are
therefore **withdrawn, not superseded** — no current number replaces them.

Claim standard, until a replacement exists: **no quality claim of any kind.**
Not SOTA, not parity, not regression, not improvement. Latency claims remain
permitted from the e2e harness. What the replacement should be is the open
question in §3 — nothing below it can be scheduled honestly until that is
decided.

---

## 2. How we supersede Zep / Mem0 / Letta / Qdrant

Not by matching feature lists. By owning a combination none of them hold, then
proving it — on a measurement that does not yet exist (§3).

| property | kern | Zep/Graphiti | Mem0 | Letta | Qdrant |
|---|---|---|---|---|---|
| Per-project self-maintaining graph (per-cwd) | ✅ | ❌ hosted | ❌ | ❌ | ❌ |
| Default recall touches no LLM (sub-ms) | ✅ | ❌ | ❌ | ❌ | n/a |
| Local-first, single binary, no network hop | ✅ | ❌ | ❌ | partial | ❌ |
| Self-forgetting (decay / stigmergy GC / cold spill) | ✅ | ❌ | partial | ❌ | ❌ |
| Graph + dense ANN + BM25 + GNN in one process | ✅ | partial | ❌ | ❌ | ❌ |
| Bi-temporal supersede off the recall path | ✅ | ✅ | ❌ | ❌ | ❌ |
| Coordinator-free CRDT federation | 🟡 building | ❌ | ❌ | ❌ | ❌ |
| Published eval numbers | ❌ withdrawn | ✅ | ✅ | ✅ | n/a |

**The three moves, in order:**

1. **Get a measurement worth steering by.** The architecture argument is won on
   paper and currently unprovable: the only scoreboard we had measured a 3B
   answerer more than it measured memory, so it was deleted (§1). We are the
   only one in this table with no published number — that is honest, and it is
   also the single biggest gap. Nothing else in this file can claim progress
   until §3's question is answered.
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

## 3. Eval — the open question everything else waits on

Everything previously listed here (items A–E: the attribution ablations, the
`--min-deliver` sweep, `--multihop-paths`, distill-coverage, judge calibration)
was scheduled against the harness deleted in `8d8b19e`. Those flags, binaries
and traces no longer exist. The items are struck rather than migrated, because
each was a probe into a composite score that has been ruled unfit for steering
memory work; re-pointing them at a new harness would import the same conflation.

**The question (blocker for every quality claim in this file):** what measures
retrieval quality without an LLM in the scoring loop?

The shape indicated by `8d8b19e`'s own reasoning — isolate the term that is
actually kern's — is a **retrieval-only** metric over a labelled corpus:
recall@k, MRR, NDCG against known-relevant ids, no answerer, no judge, so a
change in the number can only mean a change in what was retrieved. Multi-hop
becomes "were the linked entities returned", not "did a 3B model phrase it
well". Deciding behavior: **none yet — amend first.** Sub-questions the
amendment has to settle:

- (a) Where does labelled ground truth come from? Hand-labelled corpus, a
      LoCoMo-derived id-mapping (reusing the conversations while discarding the
      answer key and judge), or synthetic generation?
- (b) What is the pass bar, given there is no rival number to compare against
      once the LLM-judged scale is abandoned? A rival's LoCoMo figure is no
      longer commensurable with ours.
- (c) Does answer synthesis get measured at all, or is it explicitly out of
      scope for kern's scoreboard — given that owning it means owning a model's
      quality?

Two findings from the deleted work survive as unmeasured leads, and are the
first candidates for the replacement to check — **evidence-grade smoke only,
n≤8, not results:**

- **Multi-hop edges are the suspect, not the search.** `expand()` is a beam
  search, so "expansion is one hop" is dead. Smoke n=8: 8/8 probes had nearby
  claims, only 4/8 were linked within 2 hops → **ingest-side edge creation**.
- **Distill coverage may be the floor.** Smoke n=6: `gold_nearest_cosine` p50
  0.464, 1/6 over 0.6 — but the 0.6 bar was never calibrated (2–4-word golds
  against sentence-length claims), so it may be measuring the bar.

Shipped and retained from that era: relative-date resolution in distill
(**eval-side only — the product intake still has the gap**, see §6) and
`QueryOptions::answer_style` (eval-only by design).

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
      validate-or-remove GNN reranking. All three are measurement-gated on §3 —
      i.e. blocked until a replacement metric exists at all.
- [x] Query cache already matches paraphrases — mis-listed. `QueryCache::lookup`
      keys on cosine ≥ `theta` (0.97) against the stored query vector
      (`retrieval/cache.rs:60-71`); the exact-hash `lookup_text` path
      (`cache.rs:48`) is a pre-embed fast path, not the only key.
- [ ] HNSW tombstone compaction — dead nodes accumulate.
- [ ] No learned rerank model — every rerank is a cold LLM call.
- [ ] **A spilled kern still carries two resident indexes.** DiskANN spill is
      entity-index-only; the GNN-vector and reason-edge indexes are always
      rebuilt resident (`decisions/diskann-spill.mdx:120`). The memory ceiling is
      pushed back, not removed — this residual is not currently tracked anywhere.
- [ ] **Two freshness signals, different half-lives, neither ever tuned.** A
      24-hour one for ranking (`qbst_recency_half_life_secs`) and a 7-day one for
      retention (`base/heat.rs:18`); the offline NDCG sweep meant to tune either
      was never run (`decisions/stigmergy-over-gardening.mdx:117`). Blocked on
      §3.
- [ ] **Victim selection is O(entities) per kern per sweep, and the cold tier is
      brute-force cosine with no index** — the second scaling cliff after the
      O(N) importance scan. Previously recorded only in `FEATURES.md` §10, which
      is not a plan.
- [ ] **The self-organisation claim is unmeasured.** The convergence metrics
      (Gini over access, top-10 stability) were never built, so "the corpus
      converges on efficient paths" — a central product claim — is a design
      intention, not a measurement (`decisions/stigmergy-over-gardening.mdx:128`).
      Belongs with §3's replacement metric.
- [ ] Binary quantization stays non-user-selectable until a rescoring pass
      exists; its recall floor is too low without one (`concepts/retrieval.mdx:143`).
- [ ] Ingest queue `enqueue` detaches with no backpressure
      (`concepts/architecture.mdx:155`).
- [x] Filtered ANN end-to-end (all three seed sources on `is_active`, recall@10
      A/B `9386de0`); RRF at the answer layer; `answer:false` sub-ms no-LLM path;
      semantic query cache (cosine ≥0.97 + version stamp); lock-scoped answer
      path; workload regression trace + sweep.

---

## 5. Federation (`building`, off by default)

Phase 1 landed inline — lamport-stamped LWW on `Reason.score` and `valid_until`
(`base/merge.rs`), `PendingDelta` queue and `start_delta_flush` Delta sender.
`crdt.rs` is still 90 LoC of `GCounter` only; the LWW semantics live as inline
fields, not as named types. Fine. The OR-Set-for-`statements` plan was
**reversed, not deferred**: `id == content_hash(text)`, so importing remote
statement text both breaks content-addressing and resurrects locally-cleared
statements. Merge never imports them (`base/merge.rs:112`) and the wire target
is rejected on receipt (`gossip/handler.rs:448`), kept as a refused variant so
an older peer cannot inject text under a content-addressed id.

Missing, verified against source 2026-07-20 (re-verified during the docs audit;
three items in the previous version of this list were stale — `Fetch` is wired,
`union_statements` never existed, remote heat is no longer pinnable):

- [x] **Pulse and Question senders.** Both are live and were mis-listed as
      missing: `broadcast_pulse` and `broadcast_q` are built in `start_gossip`
      (`commands.rs:900-930`), the pulse emitter is wired into the maintenance
      tick (`:658`) and the `pulse` MCP tool (`mcp/tools_admin.rs:226`), and
      `broadcast_q` is invoked by `do_resolve` (`tick.rs:64`).
      `handle_question` is live-dispatched (`gossip/handler.rs:41`), not dead.
- [ ] **Anti-entropy.** No `AntiEntropy` wire variant. `EntitySync` ships only
      the hottest 32 per heartbeat, so cold entities may never propagate. A
      partitioned node that rejoins never catches up. (`Fetch` is single-id
      only, but it *is* live — `wire_fetch` installs the handler at
      `commands.rs:894` and the question path issues it; it is not a
      catch-up mechanism.)
- [ ] **Transport security.** Raw TCP, no TLS. `network_id` broadcast cleartext
      over UDP multicast. No signature on `GossipMessage`; `handle_conn` accepts
      any stream and `handle_peer_exchange` trusts any `msg.origin`. Needs
      `tokio-rustls` + `rcgen` as direct deps. **This one gates any deployment
      off a trusted LAN / WireGuard mesh.**
- [ ] **Deltas and pulses reach *local* rows.** The sharpest edge the docs audit
      surfaced. All four live delta targets iterate `g.all_ids()` — every kern
      including local ones — and mutate the first id match
      (`gossip/handler.rs:378-430`), with no network check. A peer that knows an
      id (they are broadcast) can therefore LWW a **local** entity's
      `valid_until` (`:424`) and its reason scores (`:403`), and inflate local
      counters under attacker-chosen replica-slot names.

      **This is not one bug, and "scope it to `remote-*`" is the wrong fix** —
      reaching local rows is *intended* for the counters. Ids are content
      hashes, so the same fact is a local row on both nodes, and
      `retrieval/score.rs:255` emits access deltas for local entities precisely
      so G-Counter slots merge across replicas. Blanket scoping kills that.
      Split by target:
      - `ValidUntil` / `ReasonScore` (LWW): an unauthenticated peer overwriting
        local truth buys nothing federation needs. Confine to `remote-*` now —
        no wire change, no dependency on §5's TLS work. Note this **subsumes
        decision (a)**: LWW-vs-max-join for `Reason.score` stops being purely a
        trust-signalling question once an untrusted writer can reach a local row.
      - Counters (G-Counter): slot-max is replay-safe by construction, so the
        exposure is only attacker-chosen *slot names*. The real fix is binding
        the slot name to an authenticated peer identity, which genuinely gates
        on transport security above. Until then it is a ranking-inflation
        nuisance, not a truth-corruption bug.

      **Sequencing, and the reason this is urgent:** the `valid_until` attack is
      armed but mostly latent — `matches_filter` only honours the field when a
      caller passes `valid_at` (`retrieval/score.rs:168`), which today only the
      MCP `valid_at` query param does. §6's "enforce `valid_until` in retrieval"
      would make expiry apply on the default path — **arming a remote
      expire-any-local-claim attack repo-wide.** Land this fix *before* that one.

- [ ] **`handle_pulse` falls back to the local root kern.** Separate and simpler:
      an unknown `pulse.kern_id` does not reject, it defaults to `g.root.id`
      (`gossip/handler.rs:319-322`), so a peer sending a garbage kern id
      deposits heat straight into your root kern — with no strength clamp
      (`tick/pulse.rs:107`). No design intent justifies the fallback. Reject
      unknown ids, clamp strength, and confine deposits to `remote-*`.
- [ ] **Backpressure.** No per-peer rate limit, no divergence metric in
      `HealthStats`. (Remote heat is no longer pinnable: entry to a `remote-*`
      kern strips heat, access counts, and confidence to neutral —
      `base/merge.rs:20`, applied at `:139`. The pin risk that remains is the
      unclamped `Pulse` strength above, which lands on *local* kerns.)
- [ ] **Entity bodies are never checked against their claimed ids.** The sync
      path accepts content up to the cap without verifying `id ==
      content_hash(text)` (`gossip/handler.rs:463`) — and content-addressing is
      the invariant every other federation guarantee rests on. It is why merge
      is safe as set-union, why a peer "cannot alter text you hold", and why
      statements are never imported. A peer can therefore file arbitrary text
      under an id that does not hash to it. Cheap to close (hash on receipt,
      drop on mismatch) and it does not need auth, unlike most of §5.
- [ ] **No Sybil defence is in effect** — and, corrected on inspection, none
      ever was. Two were written and never wired: `RateClipper`
      (the since-deleted `gossip/sybil.rs`, 175 LoC) whose `set_clipper()` had no call site in
      any commit, and `trimmed_mean_merge_hits` (`gossip/merge.rs`, 241 LoC),
      self-described as "a Sybil-resistant alternative" for fusing per-peer hit
      lists, also callerless. Both were deleted in `dc02a18` as
      verified-unreachable; the deletion changed no behaviour because neither
      had ever run. This is **unbuilt work with a reference implementation in
      git**, not a regression — a materially cheaper starting point than it
      first appeared. The layered defences from the authority design
      (edge-weight caps, pulse-coupled edge validation, temporal slashing of
      frequently-superseded producers) were never written at all.
- [ ] **Remote-injected text is retrievable and reaches an agent's context.**
      Remote entities are vector-indexed on insert, so with gossip on, recall
      output — and therefore any agent consuming it — extends to every host on
      the segment (`concepts/security.mdx:233`). Bounded by ranking-signal
      stripping, not by exclusion. Decide whether `remote-*` should be
      opt-in-per-query rather than indexed by default.
- [ ] **Two FL-derived bounds adopted on paper, neither in effect:**
      trimmed-mean / median materialisation for federated scalars (written,
      never called, deleted in `dc02a18` — see the Sybil item above; recoverable
      from git), and a provenance ledger of per-thought
      `(origin, lamport, confidence)` enabling retrospective down-weighting of a
      peer later deemed untrusted, which was never written — the shipped
      `Ledger` (`gossip/ledger.rs`) is a TTL-bounded routing cache, enough to
      know where to fetch, not who told you what
      (`decisions/knowledge-not-gradients.mdx:115`).
- [ ] One fresh TCP connection per gossip message, no pooling
      (`gossip/transport.rs:37`).

Five decisions owed before the build (deciding behavior: **none yet — amend
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

### 6a. Silent wrong answers and silent loss

Surfaced by the 2026-07-20 docs audit: each of these is documented on the site
as a known limitation but was funded nowhere. They share a failure mode — no
error, no warning, just a wrong or missing result — which is why they lead the
section. None is a missing feature; all are live defects.

- [ ] **Cold eviction drops the temporal side-map, so `as_of` silently lies.**
      A thought recovered from cold returns with `valid_from`, `valid_to` and
      `invalidated_at` unset, so an `as_of` filter treats a cold-recovered
      revision as unbounded and therefore valid at *every* instant. Point-in-time
      queries are exact over the hot graph and lossy over the cold tail, with no
      signal to the caller which they got. Documented at
      `concepts/time.mdx:112`.
- [ ] **A prose-answering reason model archives deltas having stored nothing.**
      `Some([])` from a model that replied in prose instead of JSON is
      indistinguishable from a genuine "nothing worth keeping", so the intake
      marks the delta done and moves on. Silent data loss on the main ingest
      path. Distinct from the intake-visibility item below: that one exposes
      failures, this one is not classified as a failure at all.
      (`concepts/acceptance.mdx:72`.)
- [ ] **Changing the embedding model silently zeroes recall.** Stored vectors
      stop matching query vectors; search returns nothing useful rather than
      erroring. No dimension guard, no model-identity stamp on the index, no
      startup check. (`howto/configure.mdx:39`.)
- [ ] **In-memory mode drops entities with no spill.** The spill-before-drop
      guarantee holds only for a persisted kern; with no store bound,
      `cold_spill` is skipped and the victim is simply removed
      (`concepts/heat-and-compaction.mdx:196`).
- [ ] **The cold tier is a lossy FIFO past 50k**, with no operator signal at the
      boundary — a non-durable thought you never touch, in a store that spills
      more than 50k after it, is permanently gone
      (`concepts/heat-and-compaction.mdx:191`).

### 6b. Everything else

- [x] **Validate `Kind` at the wire boundary.** Listed for a long time as "the
      highest-leverage safety fix in the repo"; it is shipped, and the premise
      was wrong on all three counts. `validate_kind` **is** called
      (`mcp/tools_mutate.rs:117`) and rejects the four internal-only kinds.
      `Superseded` is an `EntityStatus`, not an `EntityKind`
      (`base/types.rs:19-28`) — it was never claimable by anyone. And a forged
      `Fact` is unreachable regardless: the MCP path runs
      `clamp_confidence(p.conf, AGENT_SOURCE)` capping at `MAX_AI_CONFIDENCE`
      0.95, and `kind` is *derived* from confidence — `Fact` needs 1.0
      (`base/math.rs:205-210`, `base/constants.rs:62`) — so the caller's `kind`
      is discarded. Only the CLI reaches `Fact`, via
      `clamp_confidence(1.0, "user")` (`commands/ingest_cmd.rs:47`).
- [x] Clamp `conf` to [0,1] — shipped; `validate_conf` (`base/validate.rs:18`)
      is called at `mcp/tools_mutate.rs:115`.
- [ ] **Enforce `valid_until` in retrieval.** Near-dead code: `matches_filter`
      honours the field only when a caller passes `valid_at`
      (`retrieval/score.rs:168`), which today only the MCP `valid_at` param
      does, so on the default path expired claims still rank. **Blocked on §5's
      delta-scoping fix** — with gossip enabled, an unauthenticated peer can
      already LWW `valid_until` on a local row, so enforcing expiry by default
      before that lands arms a remote expire-any-local-claim attack. Order is
      not optional.
- [ ] **Source-keyed idempotency at ingest** — `(source.system, object_id,
      section)` instead of paraphrase-evadable cosine dedup.
- [ ] Require reason text on supersede.
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
- [ ] **The e2e suite is not run in CI.** `.github/workflows/ci.yml` runs
      `cargo test --workspace` only, so `e2e/` — the only thing exercising the
      real binary end to end, and the only place the hub lifecycle is
      tested — is local-only and can rot unnoticed. Needs a Python setup step
      (`e2e/requirements.txt` now declares the one dep) plus a built binary in
      the job. Judgement call deferred to a human: CI minutes and flakiness
      against a suite that currently catches things `cargo test` cannot.
- [ ] Two parallel typed transport surfaces (`kern_rpc` + `search`) with
      overlapping DTOs — one should die.
- [ ] Document gravitons truncate at the embed context window; chunk+mean-pool
      is the upgrade path. Blocked on a real document long enough to truncate.
      **Note the docs actively recommend the input that triggers this** —
      `howto/seed.mdx:43` and `concepts/stigmergy.mdx:97` both advise seeding a
      graviton with a whole document. Fix the guidance now, the truncation
      later.
- [x] Durability: `snapshot_if_dirty` on the maintenance tick. WAL rejected —
      LMDB already orders recovery.

**Operational surfaces** (documented on the site, previously unfunded):

- [ ] **`kern mcp` standalone fallback is a silent second writer** against the
      same LMDB environment — same class as the CLI/daemon race above, different
      path. Symptom: tools work but the graph never grows across sessions
      (`howto/mcp.mdx:57`).
- [ ] **Standalone `kern mcp` runs no maintenance tick and no gossip**
      (`concepts/architecture.mdx:118`), so a graph served that way never
      decays, clusters or GCs.
- [ ] **`resources/list` and `prompts/list` return `-32601` on the proxy path** —
      i.e. the normal path when a daemon is running. Advertised, non-functional
      (`howto/mcp.mdx:190`). Either forward them or stop advertising.
- [ ] **An auto-spawned daemon has no log at all** — detached with null stdio
      (`howto/install-run.mdx:167`). With hub auto-start shipped (§5x) this is
      the default posture, so the default posture is undebuggable.
- [ ] **Config validation failure does not stop startup** — logs a warning and
      continues with whatever parsed (`howto/configure.mdx:99`).
- [ ] No env-var override layer; API keys sit in plaintext TOML
      (`howto/configure.mdx:103`). `--mcp-addr` has no config field.
- [ ] RPC socket bind→chmod race (sub-millisecond, umask default) — recorded as
      an accepted risk in `concepts/security.mdx:40`; revisit only if the umask
      alternative stops being worse.

**Belief model** (`decisions/bayesian-confidence.mdx`, none funded):

- [ ] **No evidence decay.** `α` and `β` only grow, so stale consensus takes
      proportionally many new observations to unseat; tick-based `γ` damping is
      an open design (`:137`).
- [ ] **An agent cannot register disagreement at all.** There is no `Contradicts`
      reason kind and no `stance` parameter on ingest; `observe_contradict` has
      exactly one caller in the tree, GNN alignment (`:100`). Also unbuilt:
      observer-reputation weighting.
- [ ] **Supersede chains are unbounded while contested** — no `ReasonKind::Edit`
      rationale edge and no producer rate-limit, so an A/B ping-pong on one
      `external_id` grows without bound (`decisions/edit-convergence.mdx:107`).

**Documented nowhere the user would look** (funded here, missing from the site —
fix on the docs side, tracked here so it is not lost):

- [ ] `kern hub` appears on no site page, including its 1800s idle-unload
      default — a user whose daemon vanishes has nothing to read.
- [ ] Distill `kind` accuracy ~33% is not conveyed; the site presents the seven
      descriptors as a working vocabulary, so every `kind`-filtered query is
      less reliable than it reads.
- [ ] The `id` path in `query` bypasses every filter (§7a) — undocumented at
      `howto/mcp.mdx:73`.
- [ ] Automatic session intake has no producer, but `index.mdx:11` reads
      `session text → intake` as if automatic and
      `howto/intake-recall.mdx:171` tells users to "check your client hook"
      when none ships.
- [ ] Cosine dedup is paraphrase-evadable, but `howto/seed.mdx:178` calls
      re-running a seed "close to idempotent".

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
vectors / ColBERT multi-vector — re-promote any one of these **iff** §3's
replacement metric shows a retrieval-quality gap it would close.

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
