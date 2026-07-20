# LoCoMo eval — recorded baseline (2026-07-19) + validation history

> **Status: BASELINE RECORDED.** Full 3-seed run on the default local models,
> reference numbers in
> [`locomo-baseline-2026-07-19.json`](locomo-baseline-2026-07-19.json).
> Every retrieval/distill change is judged against these numbers.

## Baseline (2026-07-19)

Full locomo10 (10 dialogues, 1986 QA), seeds 0/1/2, ~1.5 h per seed on the
8 GB card. Models: `qwen3-embedding:0.6b` embed, `granite4:3b`
answer+distill (the measured default), `qwen2.5:7b` judge at temperature 0.
Built from the working tree containing the granite default fix and
`strip_think` (pre-graviton rename).

| category     |    n | F1            | ROUGE-L       | judge/abstain |
|--------------|-----:|---------------|---------------|---------------|
| single-hop   |  282 | 0.104 ± 0.004 | 0.095 ± 0.004 | 0.093 ± 0.005 |
| multi-hop    |  321 | 0.022 ± 0.002 | 0.022 ± 0.003 | 0.042 ± 0.011 |
| temporal     |   96 | 0.118 ± 0.013 | 0.109 ± 0.013 | 0.194 ± 0.016 |
| open-domain  |  841 | 0.118 ± 0.006 | 0.111 ± 0.006 | 0.194 ± 0.013 |
| adversarial  |  446 | —             | —             | 0.112 ± 0.103 |

**Overall judge+abstain: 0.137 ± 0.018** (per-seed 0.129 / 0.157 / 0.124).
Latency (seed 0, full pipeline incl. LLM synthesis): p50 901 ms, p95 1839 ms,
p99 2666 ms. Claims distilled per seed: 2213 / 2056 / 2062.

Read against the north star: Zep/Mem0-class systems publish ~0.6+ LLM-judge
scores on LoCoMo-style evals. The gap is the work, and it is now measured,
not assumed. Biggest craters: multi-hop (0.042 — reason-edge expansion is
not connecting facts across sessions) and adversarial abstention (0.112 with
10× the variance of any other category — abstention behavior is nearly
unseeded). The strict-judge caveat applies in both directions: a 7 B judge at
temperature 0 is harsher than the lenient judges that inflate published
numbers (`ROADMAP.md` §1 claim standard).

## How to test kern (the fast, reliable loop)

Three tiers. Use the cheapest one that can answer the question.

**1. Logic — seconds, no GPU.** `cargo test --features bench` (768 tests).
Covers prompts, scoring, abstention wording, cache round-trip, statistics.
Every non-LLM claim in this doc is pinned by one of these.

**2. Quality — one command, ~20 min for 300 probes.**

```
./target/debug/locomo_eval --url http://<host>:11434 \
  --samples 10 --max-qa 30 --seed 0 --concurrency 4 \
  --json --output run.json --probe-log run-probes.jsonl
```

**The canonical run uses the full pipeline.** HyDE, reranking, and synthesis
all stay on: a fast number from a disabled stage measures something kern does
not ship. `--no-hyde` / `--no-rerank` exist to attribute cost and value to a
stage, and their scores must never be quoted as kern results. Speed has to
come from making the full pipeline cheaper (fewer round trips, smaller
prompts), not from switching parts of it off.

Why these flags: `--concurrency 4` is measured fastest when the server has
`OLLAMA_NUM_PARALLEL=4` (serial takes 33 min against 22 min, because parallel
slots split GPU capacity and a serial client gets one slot). Claims are cached
per (prompt, model, seed), so re-runs skip distillation and every variant
compares over byte-identical graphs. `--probe-log` is required for tier 3.

**3. Did it actually change anything? — instant, no GPU.**

```
./target/debug/locomo_eval --compare-probes before-probes.jsonl after-probes.jsonl
```

Paired McNemar over the probes both runs answered. **Use this, not the
confidence intervals, to judge an A/B.** Pairing removes between-run variance,
so it resolves differences that overlapping CIs cannot — and it refuses to
call a wash a win. Worked example: the granite-vs-qwen embedder test scored
0.060 against 0.050, which looks like a 17% regression, but pairing showed 8
wins each way against 5 (p = 0.58) — a tie, and the embedder correctly did
not move.

Reliability properties the harness now guarantees: reports carry Wilson 95%
intervals (correct near 0, where these scores live); transport failures are
counted per phase instead of silently shrinking denominators; probe logs are
sorted by sample index so they are byte-reproducible under concurrency; and
phase wall clock is measured, not inferred from summed latencies (which count
queue wait as work and misattributed 19.9 min of "answering" inside a 21.8 min
run).

What the CI does **not** cover: LLM sampling variance across seeds, and judge
bias. For a new baseline claim, run 3 seeds — and never compare a
strict-judge number against a published lenient-judge one.

## Ablations & diagnostics (added 2026-07-20)

Implements items 0/2/5 of
[`ROADMAP.md`](../oracle/ROADMAP.md) §3:

- `--context-mode kern|grounded|grounded-retrieval` — loss attribution.
  `grounded` answers every probe from the full conversation (kern skipped,
  32 k context — rendered conversations measure 11–24 k tokens): the
  answerer+judge ceiling. `grounded-retrieval`
  answers from the 10 claims nearest the *gold answer* embedding: distill
  loss vs retrieval loss. Its report also carries `gold_nearest_cosine`
  per non-adversarial gold — the distill-coverage distribution (item 5) —
  summarized as a `gold→nearest-claim cosine` line.
- `--multihop-paths` — diagnostic, no scoring: after ingest, reports whether
  the claims nearest each multi-hop gold share any graph path within 2 hops.
  Decides item 2's fork: paths exist → deeper expansion is testable; paths
  absent → the fix is ingest-side edge creation.

Same-date eval prompt changes (both modes that call the answerer): the
short-answer style (`Answer with only the fact`) is eval-only via
`QueryOptions::answer_style`; the abstention instruction + empty-context
gate live in the product path (`retrieval/answer.rs`) and are pinned to
`locomo::is_abstention`'s marker set by a unit test.

Harness speed/precision knobs (same date):

- **Claims cache** (`eval/cache/`, gitignored): distilled claims cached per
  (distill prompt, answer model, seed) — re-runs skip the ~40 min distill
  phase, and kern vs grounded-retrieval compare over byte-identical graphs
  (paired comparison). `--fresh-distill` bypasses (no read, no write);
  `rm -rf eval/cache` refreshes.
- **`--concurrency N`** — in-flight probe and judge requests
  (Semaphore-capped tokio tasks; aggregation is index-ordered so reports
  stay deterministic). Needs `OLLAMA_NUM_PARALLEL`; grounded mode
  multiplies its 32 k KV cache per slot. Latencies at N>1 include queueing.
- **`--min-deliver F`** — retrieval delivery floor; empty delivery triggers
  the abstention gate. Default 0.0 (baseline-compatible). Note: the old
  `constants::MIN_DELIVER_SCORE=0.40` was dead code and is deleted — the
  shipped default never gated.
- **`--probe-log F.jsonl`** — appends one JSON line per probe (sample_id,
  mode, category, question, gold, pred, judge verdict, abstained,
  top_cosine): the artifact for judge calibration (item 6) and for
  calibrating the coverage cosine bar (item 5).
- **Error counters** — embed/answer/judge transport failures are counted
  and printed (`errors:` line) instead of silently shrinking denominators.
- **Cheaper judge**: `--judge-url`/`--judge-model` can point at a cloud
  model (e.g. `deepseek-v4-flash:cloud`) to remove the per-dialogue VRAM
  swap — re-judge one seed both ways before comparing against
  baseline-judged numbers.

---

## Validation history (2026-07-16)

The first end-to-end run of the harness, kept for the blocker's root-cause
record. No quality number was claimed — three probes on one dialogue is a
smoke test, not a measurement.

## What ran

```
./target/debug/locomo_eval \
  --url http://172.27.176.1:11434 \
  --samples 1 --max-qa 3 --json --output /tmp/locomo_smoke.json
```

Models (the binary defaults from `src/config/`): `qwen3-embedding:0.6b`
(embed), `qwen3.5:4b` (answer), `qwen2.5:7b` (judge). Dataset
`eval/locomo10.json`, dialogue `conv-26`.

## Result

- Pipeline validated end-to-end: **73 claims distilled and ingested**, 3 QA
  probes answered, judged, and scored. Report shape correct
  (`per_category`, `latencies_ms`, `ctx_*`, `n_queries`).
- Per-query latency: **20799 / 53776 / 33112 ms** (wall ~10m42s for 1 sample +
  3 QA). Quality at n=3 is meaningless (f1 ≈ 0.19) — scoring machinery works,
  no claim is made from it.

## Routing note (corrected 2026-07-17)

The harness ran against the WSL→Windows-host gateway IP (`172.27.176.1`).
`llm::is_local_ollama` matches the `":11434"` port marker, so the gateway URL
takes the **native** `/api/*` path — the original version of this note
(claiming `/v1` routing) was wrong.

## The blocker, root-caused (2026-07-17): it was kern, not the host

The "host cannot GPU-offload the chat models" characterization above the fold
of the original note was disproved by measurement:

- Native `complete()` hardcoded `num_gpu:0` — a serving tradeoff (a
  distillation burst must not evict the embedder + answer model from an 8 GB
  card). Because the gateway URL routes native, the eval's answerer *and*
  judge inherited that pin: kern itself forced them onto CPU. That is why
  `/api/ps` never showed them in VRAM and one-token replies took 48–57 s.
- The HTTP 500 on `num_gpu:99` was the model-default context window
  (~13 GiB with KV cache for `qwen3.5:4b`) overflowing the 8 GiB card when
  all layers were forced on — not a CUDA/driver fault. With `num_ctx:8192`
  both chat models offload fully: `qwen3.5:4b` 3.3 GiB @ 64 tok/s,
  `qwen2.5:7b` 4.8 GiB @ 53 tok/s (measured warm).

Fix: `Client::for_eval(seed)` lifts the pin for eval clients (reason calls
are the workload there) and seeds sampling; the judge is additionally pinned
to temperature 0 (measurement instrument); `eval_sample` judges in a second
phase so the 4b answerer and 7b judge swap VRAM once per dialogue instead of
twice per probe. Measured after the fix: p50 query latency 2.3 s (was
20–53 s). Serving behavior is unchanged — the `num_gpu:0` pin still protects
`/ask`.

## What this unblocks / does not

- **Unblocks (minimal):** `locomo_eval` demonstrably runs end-to-end on the
  default local models and emits a CI-diffable JSON. ROADMAP #1's old blocker
  ("run it end-to-end") is resolved at the validation level.
- **Does not unblock:** a recorded, reportable baseline number. That needs a
  GPU-offloaded host (or a cloud endpoint) and a multi-sample, multi-seed run
  with error bars, per `ROADMAP.md` §1.

## Reproduce

Build: `cargo build --features bench --bin locomo_eval`.
Run (host reachable from this WSL session at the gateway IP): the command
above. The dataset (`eval/locomo10.json`) is CC BY-NC 4.0 and never bundled;
it is present on this working tree only.
