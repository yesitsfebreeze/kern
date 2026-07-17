# LoCoMo eval — validation pass + baseline blocker (2026-07-16)

> **Status: NOT a baseline.** This records the first end-to-end run of the
> `locomo_eval` harness on the default local models, and the precisely
> characterized blocker for a real multi-sample baseline. No headline quality
> number is claimed — three probes on one dialogue is a smoke test, not a
> measurement (see `docs/aspiration.md` claim standard).

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
  with error bars, per `docs/aspiration.md` Tier 0.

## Reproduce

Build: `cargo build --features bench --bin locomo_eval`.
Run (host reachable from this WSL session at the gateway IP): the command
above. The dataset (`eval/locomo10.json`) is CC BY-NC 4.0 and never bundled;
it is present on this working tree only.
