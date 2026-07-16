# Board unblock plan — what each open ticket needs to finish

> **Historical planning document (2026-06-05).** Kept as a record of the board
> state at that date; card numbers, blockers, and code references reflect that
> snapshot and are not updated.

Research output for the `/goal` board loop (2026-06-05). Every card left in
(RE)EVALUATE is blocked on one of four things: **(A)** the eval harness (which
itself needs an external dataset + local models + a code seam), **(B)** a design
decision only the maintainer can make, **(C)** a missing prerequisite that has
to be built first, or **(D)** another author's in-flight work. This document
lists the concrete requirement for each, plus a recommended unblock order.

Status baseline: 15 tickets shipped this session; remaining 13 are below.

---

## A. The keystone — #36 LoCoMo eval harness

Unblocking #36 cascades to **5+ measurement-gated cards** (#29, #28, #47, #34,
and the tuning halves of #35/#48). It needs three things:

1. **Dataset.** LoCoMo official corpus: `snap-research/locomo` →
   `data/locomo10.json` (10 dialogues, ~300 turns / ~9k tokens each, up to 35
   sessions, multi-session QA + answer labels).
   - License: **CC BY-NC 4.0 (non-commercial)**. Eval/research use is fine;
     do NOT redistribute it inside the kern repo — load it from a path the user
     supplies (e.g. `KERN_LOCOMO_PATH`), keep it out of git.
2. **Local models.** `ollama pull bge-m3` (1024-dim embed) + `ollama pull
   qwen2.5` (reason/distill/judge). These are the kern defaults (card #35).
3. **Code seam (this is the part we can build now).** The ingest `Worker`
   hard-wires `embedder: LlmClient` (concrete; `src/ingest/worker.rs:32`), so a
   harness cannot inject a controllable embedder. Two routes:
   - **(a) Live route:** run the harness against a running daemon with the real
     models. No refactor; needs (1)+(2). Produces real quality numbers.
     Simplest path to the card's actual deliverable.
   - **(b) Seam route:** abstract `embedder: LlmClient` behind a trait/closure
     (like the query path already does: `answer::query` takes
     `embedder_fn: Option<&EmbedFunc>`). Enables a deterministic CI harness for
     *mechanics*, but fake embeddings can't measure real *quality* — so (b) is a
     testability win, not a substitute for (1)+(2).

**Harness deliverable:** a runner (bin or integration test) that, per LoCoMo
dialogue, drives capture→distill→retrieve and computes: recall@k, LLM-judge
answer score (qwen2.5 as judge), token-efficiency, and query p95 latency.

**Recommended:** route (a) with real models on a dataset path. Build route (b)
seam only if a model-free CI signal is also wanted.

---

## B. Design-decision cards (maintainer call, then implementable)

- **#55 [High] Ingest-side trust gating / quarantine.** Pick the quarantine
  representation: `(a)` `Entity.quarantined: bool`, `(b)` new
  `EntityStatus::Quarantined`, or `(c)` a trust band on `Source`. Each touches
  the 24-field `Entity` + serde defaults + every constructor + `merge_entity`
  CRDT join. Then: low-trust sources (Session/Agent auto-capture) enter
  quarantined; an independent-source `observe_support` lifts it; digest +
  retrieval skip quarantined. **Decision needed:** which representation.
- **#25 [Med] Ingest-time contradiction reconcile.** Two policies to fix:
  `(1)` ingest-latency — only invoke the LLM contradiction-judge when cosine is
  in a band ABOVE plain-similar but BELOW `dedup_threshold` (0.92), capped
  top-k, so most claims skip the LLM; `(2)` resolution — `observe_contradict`
  (reversible) vs hard `Supersede` (tick already resolves Supersedes). **Decision
  needed:** the gating band + resolution choice. Reuse `Entity::observe_contradict`.
- **#22 [Med] Bi-temporal fact modeling.** `Entity` already has `created_at`
  (txn time) + `valid_until` (valid-end); add `valid_from`. The open call is
  **temporal-aware retrieval scoring** (as-of queries, recency vs validity-window
  weighting) — needs a design + #36 to prove benefit.
- **#26 [Low] Episodic abstraction tick task.** New tick task that per
  named-cluster LLM-summarizes claims into an abstract entity linked by
  Provenance edges. **Decisions:** summary entity kind/convention + a
  dedup/refresh policy so re-summarization doesn't pile up. Needs the reason LLM.
- **#32 [Med] Tail-covering anti-entropy (federation).** New gossip sub-protocol:
  Merkle/digest set-reconciliation (exchange id-set digests, diff, pull missing).
  **Decisions:** digest granularity, reconcile cadence vs the existing
  heat-biased heartbeat, UDP message-size limits. New wire messages + handler
  state machine. (CRDT merge itself is correct — this is propagation coverage.)
- **#34 [Med] Harden chunking.** Choose: contextual-prepend (Anthropic
  Contextual Retrieval) vs enforce/verify proposition self-containment. Bounded
  sub-part that needs NO decision: harden the `paragraph_split` fallback (raw
  context-free chunks when the LLM is down) and route the descriptor hint into
  the embedded chunk text, not just the split prompt. A/B the variants in #36.

---

## C. Missing-prerequisite cards (build the prerequisite first)

- **#21 [Low] CRDT delta per-replica ownership auth.** Blocked on **the delta
  sender** (nothing emits `CrdtDeltaPayload` yet) + a defined
  `producer_id`↔node-identity mapping (a naive `delta.replica == msg.origin`
  drops legitimately relayed slots). Build the sender first, then this gate.
- **#46 [Low] `validate_fact_source` is dead.** The only caller passes the
  literal `AGENT_SOURCE`, so the Fact-tier gate is tautological. Two options:
  `(1)` thread a real per-caller auth identity into `tool_ingest` (needs an auth
  context that doesn't exist — same identity work as #21); or `(2)` delete the
  dead validator + misleading docs. **Decision needed:** for the current
  single-local-daemon (callers trusted by construction), `(2)` is correct and
  bounded NOW — only blocked on maintainer sign-off to remove a security-shaped
  (but inert) control.

---

## D. Concurrent-worker card

- **#33 [Med] Index the cold store.** Another author is actively committing this
  (`099caf9` light-projection search, `e8ba192` bench, plus uncommitted
  `vectors.bin` in the working tree). **Requirement:** let them land it; do not
  touch `src/base/cold.rs`. Mark DONE once `vectors.bin` is committed.

---

## E. Bounded refinements gated only on measurement (#36)

These are small, low-risk code changes whose only blocker is "can't verify it
helps without numbers" — they should ship together with #36's A/B:

- **#28 [Low] Min-max normalize scoring components** in `apply_boosts`
  (`score.rs`). Ranking-semantics change → A/B in #36.
- **#47 [Low] BM25 tokenizer.** Swap the hand-rolled suffix stemmer for
  **`rust-stemmers` 1.2.0** (CurrySoftware; Snowball English; zero network dep;
  input must be lowercased) + a stopword list. Note: changing the tokenizer
  changes indexed terms → existing BM25 index needs a rebuild. Measure recall
  delta in #36.
- **#29 [Med] Validate-or-remove GNN reranking.** A/B GNN vs no-GNN vs
  cross-encoder once #36 exists; remove if it doesn't clearly win.

---

## Recommended unblock order

1. **#46 decision** (cheapest; no deps) — keep or delete the dead Fact-tier gate.
2. **Provide #36 inputs** (`ollama pull bge-m3 qwen2.5`; supply LoCoMo path) →
   build #36 (route A) → run A/Bs → resolve **#29, #28, #47, #34, #22** with
   real numbers; finalize the tuned values for the already-shipped #35/#48.
3. **Design calls on #55, #25, #26, #32** → implement each.
4. **Build the delta sender** → unblocks **#21**.
5. **Let the concurrent worker land #33.**

### External resources
- LoCoMo: <https://github.com/snap-research/locomo> · paper
  <https://snap-research.github.io/locomo/> (CC BY-NC 4.0).
- rust-stemmers: <https://crates.io/crates/rust-stemmers> (1.2.0).
