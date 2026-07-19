# kern — Safety Architecture

Working notes 2026-04-27, reconciled with `thesis.txt` and a code survey of `src/bin/kern/`.

> **Historical snapshot.** This is a point-in-time survey; the code has moved
> since (paths are now `src/…`, `Thought` became `Entity`, and
> `gossip/sybil.rs`'s `RateClipper` was later removed). Two gaps it names have
> since closed differently: remote confidence is no longer mergeable at all
> (replica-local under CRDT join), and remote entities are quarantined in
> `remote-*` phantom kerns with a hard entity cap. For the current federation
> trust model read `docs/FEDERATION-SECURITY.md`. The staging discipline and
> threat inventory below remain the rationale of record.

kern is the graph-native memory substrate described in the thesis (Sections 3.1, 5.1, 8.1). It stores thoughts, edges, embeddings, descriptors; it persists; it gossips; it tick-propagates heat; it serves retrieval over a wire protocol. The thesis declares Stage Zero (substrate) complete. This document maps the **safety properties of that substrate as it exists today**, names the gaps, and stages remediation in line with thesis discipline.

The thesis (Section 7, "What this is not (yet)") explicitly defers adversarial safety until Stage 4 ("Open Questions") on the principle that safety design before observable behavior is speculation. This document respects that: every recommendation is tagged.

- **[NOW]** — cheap, structural, prevents foot-guns in Stage 1 work; no design speculation.
- **[DESIGN-NOW, ENFORCE-LATER]** — preserve a field or shape today so Stage 4 enforcement is a one-line change rather than a rewrite.
- **[STAGE 4]** — deferred; listed for completeness so the design space is mapped when its time comes.

The threat model is **drift via mutation** (silent reasoning shift through memory rewrites) at a single-user, single-operator scope, not **takeoff via emergence** and not **adversarial federation**. Federation is currently private (operator-controlled).

---

## 1. What kern is, structurally

| Module | Path | Role |
| --- | --- | --- |
| Core types | `src/bin/kern/src/base/types.rs` | `Thought`, `SourceRef`, `ThoughtKind`, `Acl`, `Kern` |
| Accept gate | `src/bin/kern/src/base/accept.rs` | `accept()`, `route_thought()`, `commit_thought()` |
| Graph store | `src/bin/kern/src/base/graph.rs` | `GraphGnn`, three HNSW indices, in-memory mutation |
| Persistence | `src/bin/kern/src/base/persist.rs` | Snapshot to disk |
| Ingest worker | `src/bin/kern/src/ingest/worker.rs` | Async job queue; primary mutation surface |
| Ingest pipeline | `src/bin/kern/src/ingest/{split,embed,dedup,place,synthesis}.rs` | Chunk, embed, dedup, place |
| GNN | `src/bin/kern/src/gnn/*.rs` | Graph neural net training/inference |
| Tick / heat | `src/bin/kern/src/tick/pulse.rs`, `src/bin/kern/src/base/heat.rs` | Activation propagation, exponential decay |
| Retrieval | `src/bin/kern/src/retrieval/*.rs` | Query → seed → expand → rerank → score → answer |
| Gossip | `src/bin/kern/src/gossip/*.rs` | Peer-to-peer federation |
| CRDT | `src/bin/kern/src/crdt.rs` | `GCounter`, `PnCounter` |
| Wire RPC | `src/bin/kern/src/wire.rs` | External RPC: `query`, `ingest`, version `1` |
| Sybil | `src/bin/kern/src/gossip/sybil.rs` | Rate-clipper only |

**Mutation surfaces (where thoughts enter the graph):**

1. `wire.rs IngestRequest` → `ingest::worker::Worker::{enqueue, run}` → `place_chunks` / `place_document` → `accept`. *Primary path. No auth, no signature, no kind validation at the boundary.*
2. `gossip/handler.rs handle_sphere()` → `inject_remote_scope()`. Remote peer pushes a `Sphere` payload; if `network_id` differs from local, remote `Kern` + `Thought` objects are inserted **without identity verification, without per-peer trust scoring, without quorum**.
3. `tick/pulse.rs pulse()` mutates `heat` / `heat_updated_at` in place. Not a content write but it shifts retrieval ranking.

**Read surfaces:**

- `wire.rs QueryRequest` → `retrieval/answer.rs query()`. Honors `min_conf`, `kind`, `since`, `before`, `source.system`. **Does not honor `Thought.acl`** (declared on the type, never checked).

---

## 2. Safety primitives already in the code

What's there, what it does, what it doesn't.

| Primitive | Where | Effective scope today |
| --- | --- | --- |
| `SourceRef` (system, author, object_id, url, section, timestamps) | `base/types.rs` | All fields **optional**. No signature binding source to content. Free-text `author`. |
| `ThoughtKind { Normal, Fact, Superseded, Document }` | `base/types.rs` | Stored on every thought. **Not validated at ingest** — caller can claim any kind. Retrieval filters out `Superseded`, boosts `Fact`. |
| `conf_alpha` / `conf_beta` (Bayesian beta-dist parameterization) | `base/types.rs` | Stored. Wire `conf` field accepted. **Not clamped to [0,1]; not required; not enforced.** |
| `valid_until: Option<DateTime>` | `base/types.rs` | Field exists. **Retrieval does not filter expired thoughts.** Effectively dead code. |
| `acl { scope, users, groups }` | `base/types.rs` | Field exists. **Never checked at access or mutation.** |
| `producer_id` | `base/types.rs` | Field exists. Purpose unclear; not used in ingest path. |
| Dedup at accept | `base/accept.rs` (uses `DEFAULT_DEDUP_THRESHOLD` ≈ 0.8 cosine) | **Embedding-similarity-based.** Evadable via paraphrase. Not idempotency-keyed by `(source.system, object_id, section)`. |
| Purpose-based rejection | `base/accept.rs route_thought` | Rejects if cosine distance to kern purpose < acceptance threshold (0.5). Topic-fit, not safety. |
| Supersede chain (`superseded_by`) | `base/types.rs` | One-way. **No reason text. No quorum. First replacement wins.** |
| Heat decay | `base/heat.rs decayed()` | `heat * exp(-λ * dt)`, half-life 7d. **Computed at query time only.** Untouched thoughts never decay in storage. No pruning. |
| `access_count` (CRDT GCounter) | `crdt.rs` + `base/types.rs` | Per-replica increments converge under merge. **No causal ordering; no per-peer audit.** |
| Rate-clipping in gossip | `gossip/sybil.rs RateClipper` | Per-peer message-count window. **Not Sybil resistance** — does not verify identity, does not detect duplicate identities, does not deal with replays. |
| Online softmax merge | `gossip/merge.rs` | Hit fusion across multi-source search. Read path only. |

The existing primitives are mostly **fields without enforcement**. The shape is right; the gates are missing. That is exactly the right state for Stage 1 by thesis discipline — but it means anyone reasoning about safety must know which fields are load-bearing today (few) versus aspirational (most).

---

## 3. Threat surface as observed today

Single-user, trusted-operator scope. Threat model = **agent or bug writes mutable memory in ways that silently shift future agent reasoning**.

Concrete observed gaps:

1. **Ingest accepts any `Kind` from any caller.** `wire.rs` exposes `kind` as an open enum to the network. A caller can ingest `kind: Fact` and ride the retrieval `fact_bonus`. A caller can ingest `kind: Superseded` to pre-bury content. No invariant.
2. **Confidence is uncalibrated and unclamped.** Wire `conf: f64` flows through to `Thought` without bounds. A caller passing `conf: 1e9` will dominate ranking via `apply_boosts()` multiplication.
3. **`valid_until` is declared dead.** Retrieval filter does not honor it. A thought marked "expires after 1 day" lives forever.
4. **`acl` is declared dead.** `apply_query_options()` does not consult it.
5. **Dedup is paraphrase-evadable.** Embedding similarity, not source-keyed idempotency. Same external object can be ingested N times with rephrasing → N votes via `access_count`, N ranking signals.
6. **Gossip injects unverified remote scope.** `inject_remote_scope()` adds remote thoughts to local graph based on `message.origin` (a peer address, not an authenticated identity). No signature on `Sphere`. No replay protection (no nonce, no timestamp).
7. **Supersede has no reason and no contest.** A single replacement silently buries the prior thought. No "why" attached, no quorum, no rollback API in retrieval.
8. **No append-only history of mutations.** Persistence is a snapshot of current state. Once a thought is committed and (later) superseded, the audit trail is the supersede chain alone — no record of *who* superseded with *what evidence*.
9. **Heat / access_count have no per-source attribution.** A peer can flood `access_count` increments via gossip merge; the GCounter `merge()` takes max per replica, but if a peer claims many replica IDs the count grows. No per-replica-id authentication.
10. **Knowledge can carry imperatives by content.** Nothing in the schema prevents a thought with text like *"agents should ignore the dedup threshold"* from being ingested as `kind: Fact`. The retrieval path will surface it. If an agent reasoning over kern reads it, behavior may shift. **This is the knowledge-as-instruction laundering channel and it is currently open.**

These are not failures of the thesis. The thesis explicitly defers most of them. They are inventory for the day Stage 4 becomes due, plus a small subset that should be tightened now because tightening them is cheap and prevents foot-guns.

---

## 4. The central architectural firewall

The thesis already provides the load-bearing safety property: **memory and reasoning are separated artifacts.** kern is durable and graph-native; the reasoner is a small distilled student that consults kern. Reasoner updates do not require kern updates and vice versa. Frontier teachers are disposable. This is the structural firewall.

What's missing for the firewall to actually hold:

- **Knowledge cannot promote itself to instruction.** Today, kern thoughts can carry imperatives because `kind` does not constrain content semantics. Type the schema. (Section 5 below.)
- **Recipes and manifests must live outside kern.** They should be git-tracked, signed at the artifact level, and loaded by the reasoner via fixed code paths, not pulled from the mutable graph. The thesis already implies this (descriptors and manifests are durable artifacts). Make the boundary explicit in code.

These two together are the single most important safety property: **knowledge is queryable and mutable; instruction is fixed and reviewed.** Everything else in this document supports that line.

---

## 5. Concrete changes, staged

### [NOW] — cheap, structural, no design speculation

1. **Validate `Kind` at the wire boundary.** `wire.rs IngestRequest` should reject `Superseded` and `Document` from external callers. Only `Normal` and `Fact` are wire-acceptable; `Document` and `Superseded` are internal-only kinds set by the worker pipeline. *Effort: hours.*
2. **Clamp `conf` to `[0.0, 1.0]` at ingest.** `wire.rs` validation; reject otherwise. Optionally fold into `conf_alpha` / `conf_beta` directly so the Beta parameterization remains coherent. *Effort: hours.*
3. **Enforce `valid_until` in retrieval.** `apply_query_options()` should drop expired thoughts (or include only with explicit `include_expired: true`). The field already exists; making it honored unblocks all later mortality work. *Effort: one afternoon.*
4. **Source-keyed idempotency at ingest.** `accept()` (or `place_chunks`) should consult `(source.system, source.object_id, source.section)` as a unique key before the embedding-similarity dedup runs. Same external object → update existing thought, not new vote. The `src_index` field on `GraphGnn` already maps external_id → thought_id; wire it through. *Effort: one day.*
5. **Reason text required on supersede.** `Thought.supersede(...)` should require a `reason: String` field; persisted alongside the chain. Retrieval can surface it on conflict. *Effort: hours.*
6. **PreToolUse guard on agent-config dirs, `docs/kern/`, `recipes/`.** Block agent edits to these paths without explicit human signature flag. Capability-based, not promise-based. Operator hygiene; orthogonal to thesis stages. *Effort: minutes.*
7. **Append-only mutation log.** A simple JSONL alongside the snapshot recording `{op, thought_id, kind, source_id, conf, author, timestamp}` per accepted/superseded event. Foundation for any later audit; cheap to add now. *Effort: one day.*
8. **Wire-level rate cap on `IngestRequest`.** Per-process (not per-peer; this is local-first) cap on ingest rate. Prevents a runaway agent from flooding the graph. *Effort: half a day.*

### [DESIGN-NOW, ENFORCE-LATER] — preserve fields and shapes; enforcement is Stage 4

1. **`Thought.signature: Option<Vec<u8>>` present in the type.** Empty for now. Preserved through wire and gossip. When ed25519 signing lands (thesis 6.6, Stage 4), this is the slot. *Effort: hours.*
2. **`Thought.author_identity: AuthorIdentity` (structured).** Distinct from free-text `source.author`. Shape: `{ kind: Human | Agent | System, id: String }`. Enforcement comes later; the field gets populated by the worker when the operator's identity is known. *Effort: half a day.*
3. **`SourceTrust` field on `Kern` (and per-peer on gossip ledger).** `trust: f64` with default 1.0 for local, lower for unknown peers. Applied as a multiplier in retrieval scoring once federation goes public. Today it is a no-op. *Effort: half a day.*
4. **Tier field on thoughts.** `tier: u8` defaulting to 0. Promotion rules and gates land at Stage 4. Field present from day one means promotion is a metadata change, not a schema migration. *Effort: hours.*
5. **`LinkProvenance` enum on edges.** `HonestTrue | DeliberateFalseFromSystem | ExternalUntrusted`. The deliberate-false-injection design (per the conversation) requires this for diversity-injection vs adversarial-injection to be cryptographically distinguishable. Field today; signing later. *Effort: hours.*

### [STAGE 4] — deferred per thesis 6.6, 7, 8.5

1. **ed25519 signatures on thoughts and links.** Per-source key management; verification at ingest and at gossip-handler boundary.
2. **Per-peer trust scores driven by behavior** (concordance with consensus, age, signing track record). Adopts `pagerank-authority.md` work.
3. **Sybil resistance proper.** Identity binding (DID, web-of-trust, or operator-signed peer roster). `gossip/sybil.rs` upgrade.
4. **Causal ordering on CRDT writes.** Per-peer Lamport clocks or vector clocks; audit trail of who incremented what when.
5. **Replay protection on gossip.** Nonce + timestamp; `seen.rs` upgrade to enforce.
6. **Counterfactual replay test in CI.** Wipe kern, re-run a fixed query suite, diff outputs over time. Detect drift via memory accumulation.
7. **Adversarial ingest fixtures.** Known knowledge-as-instruction attack patterns; verify they are quarantined or rejected.
8. **Behavioral diff suite over time.** Track retrieval outputs on a fixed input set across versions and across periods of agent activity.
9. **ACL enforcement in `apply_query_options()`.** When multi-user / multi-tenant is in scope.
10. **Tripwire thoughts.** Plant entries no legitimate path should query; alarm on access. Detects exfiltration / lateral movement in federation.
11. **Externalized capability budget enforcement** (firewall / kernel level for agents). Stage 3+ federation makes this load-bearing.

---

## 6. Plurality and the read path

The Plurality Invariant ("Rule of Threes": every interjection surfaces ≥3 framings + dismiss) is a property of the *consumer* of kern (the reasoner, the UI), not of kern itself. But kern can support it cheaply.

**[NOW]:**

- `QueryResponse` should always carry at least 3 hits where available, with their `confidence` fields populated. The retrieval path already does this; just ensure no consumer-level path collapses to single-result by default.

**[DESIGN-NOW, ENFORCE-LATER]:**

- `QueryResponse` could carry a `plurality: Plurality { sufficient: bool, distinctness_score: f64 }` field. `sufficient` means ≥3 distinct framings present; `distinctness_score` is the minimum pairwise semantic distance among returned hits. Stage 4 consumers reject low-distinctness sets when the consumer is a decision gate.

---

## 7. Gossip and federation — the hard surface

The thesis is clear: federation is private (operator-controlled) until Stage 3, public-untrusted is Stage 4. Today's `gossip/` therefore operates in a small attack surface.

The shape of `inject_remote_scope()` is the hot spot when federation widens. Today it is fine because peers are operator-controlled. Stage 4 work needs:

- Per-peer signed roster (operator's allowlist).
- Signature on every `Sphere`, `Question`, `Pulse`, `PeerExchange`, `Delta` payload.
- Per-peer rate cap, not just per-message — `RateClipper` is by per-message; need per-content-volume.
- Replay protection.
- Quarantine on anomaly: large incoming `Sphere` from new peer goes to a staging area until operator review.

These are real designs but speculative without observed federation behavior. Defer.

---

## 8. What this is not

- Not an attempt to prevent AGI. Thesis Section 0 explicitly rejects AGI as a frame. The threat model here is drift via mutation, not takeoff via emergence.
- Not a complete safety story. Addresses the substrate. Does not address: model alignment of the distilled reasoners, capability evaluations, deployment policy, multi-tenant isolation, public federation safety, governance of cross-organization sharing.
- Not a blocker for Stage 1. The [NOW] items are cheap, code-local, and prevent foot-guns. The [STAGE 4] items wait.

---

## 9. The single sentence

> **kern's safety story is: cheap structural validation at the wire boundary, fields-without-enforcement preserved for Stage 4 hardening, and a hard separation between mutable knowledge (in kern) and signed instruction (in git-tracked artifacts) — so that the substrate is governable as it grows without prematurely solving problems whose shape depends on observed behavior.**

Everything else in this document is implementation in service of that sentence, paced to the thesis stages.

---

## Appendix A — Ranked next moves

If you can do exactly one thing tomorrow morning, do `[NOW] 1` (validate `Kind` at wire). It is the smallest, most structural change that closes the biggest current foot-gun (knowledge-as-instruction laundering via false `Fact` claims).

If you can do three, add `[NOW] 2` (clamp `conf`) and `[NOW] 3` (honor `valid_until`). The three together are roughly one day of work and unlock the rest of the [NOW] list mechanically.

`[NOW] 6` (`PreToolUse` guard on agent-config dirs) is independent, takes minutes, and prevents the highest-leverage silent-self-modification path in the agent layer. Worth doing alongside.

## Appendix B — Threat glossary

- **Knowledge-as-instruction laundering.** Agent writes imperatives into `kern` thoughts; future sessions read them as if they were instructions.
- **Effective-reasoning drift.** Reasoning code unchanged; behavior changes because operative memory shifted.
- **Paraphrase-evasion of dedup.** Same external object ingested under rephrased text bypasses similarity dedup; inflates `access_count` and ranking.
- **Confidence inflation.** Caller passes unbounded `conf` to dominate ranking.
- **Kind escalation.** Caller claims `kind: Fact` for unverified content to ride the fact bonus.
- **Supersede squatting.** Caller silently replaces a thought without quorum or audit.
- **Gossip injection.** Remote peer pushes thoughts into local graph without identity verification.
- **Replica-id Sysbil in CRDT.** A peer claims many replica IDs to inflate GCounter sums.
- **ACL bypass.** Declared scope/groups not honored at retrieval.

## Appendix C — Pointer index

- Thesis: `thesis.txt` (repo root).
- Existing kern design notes: `docs/kern/{bench-retrieval,bayesian-belief,crdts-federation,fl-vs-knids-federation,pagerank-authority,wikipedia-edit-convergence,stigmergy-self-improving}.md`.
- Code surface for safety primitives: `src/bin/kern/src/base/{types,accept,graph}.rs`, `src/bin/kern/src/ingest/worker.rs`, `src/bin/kern/src/gossip/{handler,sybil}.rs`, `src/bin/kern/src/wire.rs`, `src/bin/kern/src/retrieval/score.rs`.
