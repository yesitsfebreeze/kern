# LoCoMo improvements — closing the 0.137

Derived from the recorded baseline
([`locomo-baseline-2026-07-19.json`](locomo-baseline-2026-07-19.json),
overall judge+abstain **0.137 ± 0.018** vs the Zep/Mem0-class ~0.6+ the north
star names). Ranked by leverage. Each item: the measured crater, the
hypothesis, the code anchor, the experiment, and what number must move.
Nothing here is a claim — every hypothesis dies or survives against a seed-0
re-run (`docs/aspiration.md` claim standard).

## 0. Decompose the loss first (the avoided question)

**Crater:** all of them. 0.86 of headroom is lost somewhere between distill,
retrieval, and synthesis — and every fix below guesses differently about
where.

**Experiment (one day, no product code):** three ablation runs on seed 0:

1. **Grounded context** — answer each probe with the *full conversation* in the
   prompt (skip kern entirely). Scores the answerer+judge ceiling; everything
   below it is kern's loss.
2. **Grounded retrieval** — answer with the top-10 *gold-relevant* claims
   (nearest by embedding to the gold answer). Splits distill loss ("the fact
   never became a claim") from retrieval loss ("the claim exists but wasn't
   recalled").
3. **Baseline** — the recorded 0.137.

The three deltas dictate where the next month goes. Harness change: a
`--context-mode grounded|grounded-retrieval|kern` flag on `locomo_eval`.

**Success:** not a score — a signed attribution table in this file.

**Status (2026-07-20): harness landed.** Both ablation modes wired
(`locomo_run.rs`); grounded mode skips ingest and answers from the full
conversation at 32 k context (rendered conversations measure 11–24 k
tokens — the default 8 k window, and a first-guess 16 k, both truncate and
measure recency instead of the ceiling; caught in the smoke run);
grounded-retrieval embeds the gold answer and answers from the 10 nearest
claims. Attribution runs pending.

## 1. Seed abstention (cheapest single-category win)

**Crater:** adversarial 0.112 ± 0.103 — 446 probes (22% of the benchmark),
near-zero score, 10× the variance of any other category.

**Hypothesis:** the capability exists but is never requested. The answer
prompt (`answer_prompt_from`, `src/retrieval/answer.rs:281`) says "Answer the
question concisely using only the context above" — it never says *decline
when the context lacks the answer*. granite4:3b went 4/4 on declining in
`scripts/answer_bench.py` when asked. (Corrected 2026-07-20: the original
claim that "`MIN_DELIVER_SCORE=0.40` already gates delivery" was false —
that constant was dead code, `RetrievalConfig::default` ships
`min_deliver_score: 0.0`, so delivery was ungated and the empty-context
gate near-never fires once claims exist. The constant is deleted;
`locomo_eval --min-deliver` now exposes the floor for the sweep.)

**Fix (two lines + one gate):**
- Prompt: append "If the context does not contain the answer, say exactly:
  I don't have information about that."
- Gate: if `retrieved.results` is empty (or top score < floor), skip the LLM
  and return the abstention string directly — cheaper *and* more reliable.
- Keep the emitted string inside `locomo::is_abstention`'s marker set
  (`src/bench_support/locomo.rs:133`) — and grep that marker list against the
  prompt wording in a unit test so they can't drift apart.

**Success:** adversarial ≥ 0.5 with variance collapsing, no regression in the
other four categories (the risk: over-abstaining on answerable probes —
that's why the non-adversarial categories are watched in the same run).

**Status (2026-07-20): landed.** Prompt instructs the exact string
(`answer::NO_ANSWER`), empty-context synthesis returns it without an LLM
call, and `locomo.rs` pins both to the `is_abstention` marker set
(`answer_paths_abstention_wording_stays_inside_the_marker_set`). The
delivery floor is sweepable (`--min-deliver 0 / 0.2 / 0.4` on seed 0) —
that sweep is the missing half of this item. Re-run pending.

## 2. Multi-hop: expansion is one hop deep

**Crater:** multi-hop 0.042 ± 0.011 — the worst category, on the axis kern
claims as its structural advantage (VISION: "recall can walk reason edges").

**Fact (corrected 2026-07-20 — the original claim was wrong):** `expand()`
(`src/retrieval/expand.rs:178`) is a beam search, not a one-hop walk: popped
neighbors are themselves expanded, so A→B→C is structurally reachable. The
real bounds are the beam (`max_expansions=500`) and the decay threshold
(`score < global_best * 0.25` prunes the chain) — and, upstream of both,
whether the edges exist at all.

**Second suspect:** at ingest, claims get only a `Similarity` edge to the
nearest neighbor and `Provenance` to the source doc
(`commit_entity`, `src/base/accept.rs`). Cross-session facts about the same
entity ("Caroline") connect only if cosine-near — no entity-identity linking.
So even a deeper walk may find no path to walk.

**Experiments, in order:**
1. Count first: for 20 multi-hop probes, do gold-supporting claims share any
   path ≤ 2 hops? If not, deeper expansion is pointless — the edges don't
   exist, and the fix is ingest-side (entity co-mention linking at distill:
   claims extracted from the same distill pass about the same subject get a
   `Related` edge).
2. If paths exist: second expansion wave over the top-N first-wave neighbors
   (bounded: N=8, one extra hop, reuse the existing `PathChain` scoring —
   no new machinery).

**Success:** multi-hop ≥ 0.15 (≈4×) without p50 latency leaving the
low-single-digit ms for the graph phase.

**Status (2026-07-20): experiment 1 wired, fix gated on its result.**
`locomo_eval --multihop-paths` ingests, then reports per multi-hop probe
whether the 3 claims nearest the gold share any path ≤ 2 hops
(`run_multihop_paths`, traversal semantics identical to `expand()` via
`expand::neighbor_ids`). Given the corrected fact above, a deeper walk is
NOT the default fix — if the diagnostic shows unlinked claims, the lever is
ingest-side edge creation; if linked, it's beam/threshold tuning.
Smoke (conv-26, first 8 multi-hop probes): 8/8 had ≥2 nearby claims,
4/8 linked within 2 hops — both levers live at n=8; the full-scale run
decides the split.

## 3. Temporal: resolve relative dates at distill

**Crater:** temporal 0.194 ± 0.016, and the smoke misses were all date
questions (gold "7 May 2023", pred a different date; gold "2022", pred
"last year").

**Hypothesis:** LoCoMo sessions are timestamped and speakers say "last week"
/ "last May". The distill schema already carries `valid_from`
(`src/ingest/distill.rs`) but the conversation's session dates aren't fed to
the prompt, so relative references either drop or distill verbatim
("painted a sunrise last year" — unanswerable against gold "2022").

**Fix:** prepend each session's date header to the distill input and instruct
the model to emit absolute dates in claim text + `valid_from`. Eval-side this
is `distill_locomo` (`src/bench_support/locomo_run.rs:197`); product-side the
capture intake has the same gap with session timestamps.

**Success:** temporal ≥ 0.35; spot-check 10 date claims carry absolute dates.

**Status (2026-07-20): landed (eval side).** The session date header was
already prepended per session; the distill prompt now instructs resolving
every relative date against it and writing the absolute date in the claim.
`valid_from` emission is deliberately not requested: the eval worker path
drops it (`Worker::run` carries no valid-from), so it would be dead prompt
weight for a 3 B model — plumb it when bi-temporal scoring needs it.
Product capture intake still has the gap.

## 4. Answer shape: F1 is measuring verbosity, not knowledge

**Crater:** F1 ~0.10 everywhere — even where the judge says 0.19 correct.
Golds are 2–4 words ("7 May 2023", "Psychology, counseling certification");
kern answers full sentences ("Caroline is likely to pursue education in
fields related to counseling, mental health work, or…").

**Fix:** eval-context prompt tweak: "Answer with only the fact — a few words,
no sentence." Token-F1 rises mechanically; judge may too (less room to hedge).
This is presentation, not memory — do it *after* 0–3 so it doesn't mask real
movement, but before quoting any F1 externally (published LoCoMo numbers use
short-answer style; ours are handicapped in comparison).

**Success:** F1 roughly doubles with judge flat-or-up.

**Status (2026-07-20): landed, eval-only.** `QueryOptions::answer_style`
carries the hint into synthesis; the eval sets
`Answer with only the fact — a few words, not a full sentence.` The product
prompt is untouched. Sequencing honored: the style rides every mode, so 0's
ablations and 4's shape change land in the same measured run — deltas
between modes stay style-invariant.

## 5. Distill coverage: did the fact ever become a claim?

**Crater:** unknown share of every category (decomposition in 0 sizes it).

**Check:** ~2,100 claims per seed from 10 conversations. For each of the 1,540
non-adversarial golds, nearest-claim cosine against the gold answer — the
distribution's low tail is the set of facts distillation dropped. Free to
compute from existing artifacts (claims are in the eval graph; golds in the
dataset).

**Lever if the tail is fat:** distill per session instead of one pass over the
whole conversation (`distill_locomo` feeds the entire dialogue to one
prompt — long-context claim extraction is exactly where small models drop
facts; the product intake already processes per-delta, so per-session eval
distill also makes the eval *more* representative).

**Success:** ≥90% of golds have a claim at cosine ≥ 0.6, or item 0's
grounded-retrieval delta shrinks accordingly.

**Status (2026-07-20): measurement lands with item 0's grounded-retrieval
run** — that mode already embeds each gold and searches the claim index, so
the report records `gold_nearest_cosine` per non-adversarial gold and the
summary prints p10/p50/p90 + the ≥ 0.6 share. Per-session distill (the
lever) was already in place (`ingest_sample` distills per session, not per
conversation). Smoke (conv-26, n=6): p50 0.464, ≥0.6 only 1/6, and the
identity probe's true claim didn't surface in the gold's top-10 — but read
the threshold with care: golds are 2–4 words and claims are sentences, so
the cosine ceiling sits below document-to-document similarity. Calibrate
the 0.6 bar against a handful of known-covered golds before trusting the
tail at full scale.

## 6. Judge calibration (measurement hygiene, not a score lever)

The strict 7B judge marked "counseling, mental health work" wrong against
gold "Psychology, counseling certification" in the smoke. Before trusting
category deltas < ~5 points: hand-label 50 judged verdicts, compute judge
agreement. If < 0.9, tighten `judge_prompt` (explicit "semantically
equivalent counts as correct"). Never compare kern's strict-judge numbers
against published lenient-judge numbers without saying so.

---

**Sequencing:** 0 first (it re-prioritizes everything below), then 1
(mechanical, isolated), then 2/3 in either order (biggest craters), 4 before
any external quote, 5 informed by 0, 6 continuous. Re-run seed 0 after each
landed item; full 3-seed only when claiming a new baseline.
