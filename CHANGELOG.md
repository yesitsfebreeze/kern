# Changelog

- 2026-07-17 — Commentary lives in splinter notes, not in source. The whole
  tree was swept: informational comments (rationale, history, design
  narrative) migrated into per-file `.splinter/**/*.splinter.md` notes —
  durable agent memory that survives re-splits, committed via a gitignore
  carve-out — and inline comments remain only where load-bearing (safety,
  lock ordering, invariants, units, workarounds with a reason). Pure noise
  (restating code, section banners, commented-out code) deleted outright.
  Going forward new commentary follows the same split: sidecar note by
  default, inline only for constraints code cannot express. Tradeoff:
  rationale now lives one hop from the code and needs splinter (or the raw
  `.splinter/` tree) to read — accepted, because sidecar notes survive
  rewrites while inline comments rot with the line they sit on. Upstream
  behavior amendment (comments-last-resort gains the sidecar rule) is staged
  in `.scratch/oracle-behavior-amend.md`; this session's write-scope hook
  blocks `/home/feb/dev/oracle`, so applying it is a user step. Decided by:
  comments-last-resort, delete-superseded.
  Supersedes: inline design-narrative comments across `src/`.

- 2026-07-17 — The capture drop-dir is named the **intake**; the interim
  print-queue-style working name it shipped under is scrubbed from the
  entire tree — code (`ingest::intake`, `intake_direct`, tracing target
  `kern.ingest.intake`), hook internals (`MAX_INTAKE_FILES`,
  `intakeEvictions`), docs, agent briefs, and splinter notes, with no alias
  or historical mention kept anywhere (git history remains the only record).
  The MCP `ingest` durable ack status is now `"accepted"` (HTTP 202
  semantics: persisted, processed later). On-disk layout untouched —
  `.kern/capture/`, `direct/`, `done/` keep their names, so nothing
  migrates. Tradeoff: any external client matching the old ack string
  breaks, and future readers must consult git history to trace the old
  vocabulary; accepted — the only shipped consumers (kern hooks) don't read
  the ack, and the old name was never meant to ship.
  Decided by: delete-superseded, name-the-tradeoff.
  Supersedes: the previous capture-queue vocabulary everywhere (code,
  hooks, docs).

- 2026-07-17 — Durability primitive: snapshots first; ROADMAP #4 closed. The
  primitive is `snapshot_if_dirty` on the maintenance tick — a
  mutation-epoch-gated guarded full flush reusing `flush_guarded` verbatim
  (no-op when the epoch hasn't moved). Tradeoff: up to one tick interval
  (60 s) of derived-state loss is accepted — heat/access stamps stay
  epoch-exempt by design — in exchange for zero new recovery code; a WAL in
  front of LMDB would duplicate LMDB's own journal, add a persisted op enum
  to the append-only surface, and introduce replay-ordering semantics the
  state-based CRDT merge deliberately avoids (a stale WAL replayed after a
  gossip merge could resurrect superseded entities). Along the way, two tick
  tasks were leaking durability: `do_cluster` rewrote the parent kern without
  its migrated entities while never persisting the spawned child — a crash
  there permanently erased already-durable entities (destructive, not a
  window; now child-first Persist, proven by a crash test that fails on the
  old code) — and `do_seed_questions` minted edges with no Persist at all.
  Loss window after: ≤ 1 tick for epoch-bumping state, zero for cluster
  migrations and seeded questions, per-job for ingest
  (unchanged). Decided by: name-the-tradeoff, fix-bugs-on-sight, verify-before-claiming.
  Supersedes: the crash-lossy tick tasks and the "neither primitive exists"
  framing of ROADMAP #4.

- 2026-07-17 — HNSW insert is id-stable; ROADMAP #5 closed. Root causes of
  nondeterminism: node levels drawn from a positional RNG stream (nth insert
  ate the nth draw), HashMap iteration feeding insert order on every index
  rebuild, and distance-only tie-breaking. Fixed at the root: levels are now
  a pure function of the id (FNV-1a → exponential), rebuilds iterate ids in
  sorted order, ties break on (distance, id); `structure_digest()` is the
  determinism contract surface. Proven per verify-before-claiming: two new
  tests failed on the old code (level-vs-insert-order, cross-instance rebuild
  digest) and pass now; recall@10/NDCG@10 bit-identical before/after; latency
  and throughput deltas within run-to-run noise, so no speed claim is made.
  Tradeoff: O(n log n) id sorts per rebuild and hash-derived levels
  marginally less statistically clean than a PRNG stream — accepted for
  determinism at zero measured quality
  cost. Decided by: verify-before-claiming, fix-the-root, name-the-tradeoff.
  Supersedes: the RNG-seeded level path and unordered rebuild iteration.

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
