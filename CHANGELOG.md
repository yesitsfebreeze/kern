# Changelog

- 2026-07-17 — Root-caused the eval "GPU blocker": it was kern, not the host.
  The WSL gateway URL matches `is_local_ollama`'s `":11434"` marker, so eval
  traffic took the native path — where `complete()` hardcoded `num_gpu:0` (a
  serving tradeoff protecting `/ask` from distillation bursts) and forced the
  eval's answerer and judge onto CPU. Measured after the fix: `qwen3.5:4b`
  64 tok/s and `qwen2.5:7b` 53 tok/s, each fully VRAM-resident at
  `num_ctx:8192`; the earlier HTTP 500 on `num_gpu:99` was the model-default
  context (~13 GiB KV cache) overflowing the 8 GiB card, not a driver fault.
  Changes: `Client::for_eval(seed)` puts reason calls on GPU and seeds
  sampling (serving default untouched); `with_temperature` pins the judge to
  0 — the judge is the measurement instrument, its verdicts must not carry
  sampling noise, while the answerer/distiller keep default temperature
  because their sampling variance is what multi-seed error bars measure; the
  eval judges in a second phase per sample so the 4b answerer and 7b judge
  swap VRAM once per dialogue instead of twice per probe (measured p50 query
  latency 2.3 s, down from 20–53 s). Tradeoff: serving still pins reason to
  CPU — a distillation burst on an 8 GB card must not evict the answer path;
  eval flips the pin because there reason IS the
  workload. Decided by: fix-the-root, verify-before-claiming, name-the-tradeoff.
  Supersedes: the 2026-07-16 blocker characterization ("host cannot
  GPU-offload the chat models") and `docs/kern/eval-locomo.md`'s routing note
  claiming gateway traffic uses `/v1`.

- 2026-07-17 — Surveyed the competitive landscape and recorded it
  (`docs/landscape.md` + `landscape` specialist): Zep/Graphiti, Mem0, Letta,
  Cognee as the closest overall set; YourMemory as the direct decay+LoCoMo
  rival; mnemo and AgentDB/ruvector on the Rust/embedded axis; no shipped
  competitor on CRDT federation. The doc states feature-level position only —
  no quality ranking until the ROADMAP #1 baseline
  exists. Decided by: record-the-decision, verify-before-claiming.
  Supersedes: the bare
  competitive-set line in `VISION.md` as the place comparisons start from
  (the line stays; the doc carries the detail).

- 2026-07-16 — GitHub Pages enabled and self-healing: the site 404'd because Pages was never enabled on the repo (`gh api .../pages` → 404) and `actions/configure-pages@v5` defaults to `enablement:false`, so the lone deploy hard-errored. Enabled Pages via the API (`build_type: workflow`) and set `enablement:true` in `.github/workflows/pages.yml`; the deploy now succeeds (HTTP 200). Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-16 — Validated `locomo_eval` end-to-end on the default local models (1 sample / 3 QA; `docs/kern/eval-locomo.md`). The pipeline runs and emits a CI-diffable JSON; no baseline number is claimed — n=3 is a smoke test, not a measurement. The real blocker for a recorded baseline is characterized precisely: the host runs the chat models (`qwen3.5:4b`, `qwen2.5:7b`) on CPU (~50 s per one-token call; `/api/ps` shows only the embed models in VRAM), so the full ~1990-probe run would measure CPU-bound generation, not the configured models. ROADMAP #1's blocker updated accordingly. Decided by: verify-before-claiming. Supersedes: the old ROADMAP #1 blocker ("run `locomo_eval` end-to-end with the default local models, multi-seed … and commit the reference JSON").

- 2026-07-16 — Removed VOIT from the repo (the `.voit/` runtime dir + the VOIT-only `AGENTS.md`). Nothing in the build or tests referenced it; the oracle content files (VISION/FEATURES/ROADMAP/CHANGELOG/SPECIALISTS) plus the pre-commit hook are the project's process machinery, and the VOIT role/workflow files were a second, drifting set whose onboarding contract pointed at files that did not exist. Decided by: delete-superseded. Supersedes: the VOIT onboarding contract formerly in `AGENTS.md`.

- 2026-07-16 — Added `just insight` (`scripts/insight.py`): a measured repository snapshot (build, test count, code shape, oracle state, baseline presence) so project status is a run, not a recollection. Composes existing tools (cargo, nextest, tokei, git) rather than building analysis machinery. Decided by: verify-before-claiming, builtin-before-built. Supersedes: nothing.

- 2026-07-16 — Initialized the content files from the source tree: `VISION.md` (failable criteria distilled from `docs/vision.md` and `docs/aspiration.md`), `FEATURES.md` (present state, federation and the eval harness marked `building`), `ROADMAP.md` (seven open questions, eval baseline first), `SPECIALISTS.md` (seven delegation briefs by subsystem). Decided by: record-the-decision. Supersedes: nothing — first content.
- 2026-07-16 — Pinned the initial behavior set, ten from upstream `v1`, `verify-before-claiming` heaviest — measure-don't-assume is already this repo's loudest law (`docs/aspiration.md` claim standard). Decided by: the oracle. Supersedes: the empty pin list from install.
- 2026-07-16 — Installed the oracle: `ORACLE.md` is this repository's process machinery from here on. Decided by: the oracle. Supersedes: nothing.
