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

## Routing note

The harness ran against the WSL→Windows-host gateway IP (`172.27.176.1`).
`llm::is_local_ollama` matches `localhost`/`127.0.0.1`/`ollama` literals, so a
gateway IP routes chat/embed calls through ollama's OpenAI-compat `/v1/*` path
instead of native `/api/embed`. Ollama serves both, so it works — but the
intended native path (with `num_ctx`/`keep_alive`/`num_gpu` tuning) is not
exercised this run. To get the native path from WSL, either bind a
`localhost:11434` forward to the host or teach `is_local_ollama` the gateway.

## The real blocker (not kern, not the routing)

The host **cannot GPU-offload the chat models** — confirmed, not assumed:

- `/api/ps` shows only the two embedding models resident in VRAM
  (`size_vram == size`); `qwen3.5:4b` and `qwen2.5:7b` never land in VRAM.
- Forcing `num_gpu:99` on `qwen3.5:4b` via native `/api/chat` returns
  **HTTP 500** (server error, empty reply) after ~35 s — GPU offload fails.
- `/api/show` on all three models carries **no `num_gpu` pin**, so this is not a
  kern/Modelfile config forcing CPU; it is the host's GPU stack refusing the
  larger offload. The 0.6 b embedder offloads fine; the 4 b / 7 b do not — most
  likely they exceed free VRAM, or a CUDA/driver fault on the larger offload.
  A one-token `/v1/chat/completions` reply (CPU fallback) is 48–57 s vs <2 s on
  GPU.

Remediation is on the Windows host: free VRAM / check the ollama log behind
the 500 / update the GPU driver — not a kern change. Once the chat models
offload, the full ~1990-probe run drops from ≈11–27 h (CPU) to ≈1.7 h (GPU) and
the numbers measure the configured models, not CPU-bound generation.

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
