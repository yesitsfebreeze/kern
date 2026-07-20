

# Changelog

- 2026-07-20 — Eval results carry their own uncertainty, and A/B becomes a
  command instead of a habit. New `bench_support::evalstats` provides a
  Wilson score interval (correct near 0, where every category here lives —
  the normal approximation returns impossible bounds at p≈0.05) and an
  exact two-sided McNemar test. `EvalReport::summary` prints a 95% CI per
  category and overall; `locomo_eval --compare-probes A.jsonl B.jsonl`
  pairs two runs over the probes both answered and reports the delta, the
  discordant split, the p-value, and a verdict that refuses to call a wash
  a win. Cause: every comparison this session was eyeballed from point
  estimates — the granite-vs-qwen embedder A/B reads as 0.060 vs 0.050 (a
  17% regression) but pairs to 8-5 discordant, p = 0.58, a tie. Pairing
  removes between-run variance and resolves what overlapping CIs cannot,
  so the summary names the right tool for the job. `docs/kern/eval-locomo.md`
  documents the three-tier loop (cargo test → one eval command → compare)
  and records that `--concurrency 4` is measured fastest once the server
  has `OLLAMA_NUM_PARALLEL=4` — serial takes 33 min against 22 min,
  because parallel slots split GPU capacity and a serial client gets one.
  Tradeoff, named: the interval covers sampling error only, not LLM
  sampling variance or judge bias, and the output says so to stop it being
  over-read.
  Decided by: verify-before-claiming (a score without an interval invites reading noise as signal), record-the-decision (the A/B procedure is executable now, not folklore). Supersedes: ad-hoc significance checks and bare point-estimate comparisons.

- 2026-07-20 — `EvalReport` records wall clock per phase
  (`sample_phase_secs`, `judge_phase_secs`) and the summary prints them
  next to the summed query latency. Cause: after deferred judging landed,
  the answer/judge split had to be *inferred* from summed latencies, and
  that number counts queue wait as work — under `--concurrency 4` it read
  19.9 min of "answering" against a 21.8 min total run, which is
  uninterpretable. Phases are timed at the top level, not summed per
  sample, because concurrent samples overlap and summing double-counts.
  Decided by: verify-before-claiming (an optimization loop needs measured
  phases, not inferred ones). Supersedes: inferring phase cost from
  `latencies_ms`.

- 2026-07-20 — vLLM is ruled out for the Granite 4 answerer on this
  hardware, and the reason is a vLLM bug rather than a tuning failure:
  `KeyError: 'full_attention'` during KV-cache setup for
  `GraniteMoeHybridForCausalLM`, reproduced identically under two
  unrelated quantization paths (fp8 and bitsandbytes 4-bit) and with
  `--enforce-eager`. The architecture is in vLLM 0.25.1's supported
  registry but crashes at engine init. bf16 is not an escape: 6.8 GB of
  weights against 6.98 GB free leaves nothing for KV cache. Recorded so
  this is not re-derived: `ibm-granite/granite-4.0-micro` is a byte-exact
  param match (3,402,836,480) for Ollama's `granite4:3b`, kern needs no
  code change to drive vLLM (`--answer-url .../v1` already routes
  OpenAI-compat), and `uv` is required to build the venv since
  `python3.12-venv` is absent and sudo needs a password. Tradeoff, named:
  vLLM's continuous batching genuinely beats Ollama under concurrent load,
  but the answer path was measured at 7.2 of 24.9 min, so its ceiling here
  was ~1.4× by Amdahl — the judge scheduling was the real lever, and that
  is already fixed. Decided by: verify-before-claiming, name-the-tradeoff.
  Supersedes: the assumption that vLLM was an available speed lever.

- 2026-07-20 — Judging moves to one global phase after every dialogue has
  answered (`judge_all`), instead of a per-dialogue judge pass. Measured
  cause: in the seed-0 embed comparison, wall clock was 24.9 min of which
  only **7.2 min was the answer path** — the other 17.7 min (71%) was the
  judge, a 7B model swapping VRAM against granite on one 8 GB card once per
  dialogue. Judging once means the judge model loads once per run. This also
  answers the "should we use vLLM for the answerer" question with a number:
  optimizing the answerer targets 29% of wall clock, so by Amdahl it caps
  total speedup near 1.4× — the judge was always the bottleneck. Supporting
  cleanups: `ProbeCtx` drops its now-unused judge handle; probe records are
  sorted by sample index before logging (samples finish out of order under
  concurrency, and a reproducible probe log is the point of the artifact);
  the adversarial category number is now the single constant
  `locomo::ADVERSARIAL_CATEGORY` instead of a magic `5` in three places.
  Also repaired a non-compiling tree (`all_records` type mismatch) left in
  the concurrency work this change rewrites.
  Decided by: verify-before-claiming (profile before optimizing), fix-the-root (judge
  scheduling, not per-call tuning), delete-superseded (the magic 5, the dead
  judge handle). Supersedes: per-dialogue two-phase judging.

- 2026-07-20 — The embedder stays `qwen3-embedding:0.6b`; unifying every
  default onto the granite family is **not** funded by measurement. Paired
  seed-0 comparison (10 dialogues × first 30 QA = 300 probes, identical
  2146 cached claims so only the embed model differed):
  qwen 0.060 vs `granite-embedding:278m` 0.050 overall, and McNemar on the
  per-probe verdicts gives 8 qwen-only vs 5 granite-only wins,
  **p = 0.58** — a tie, not a granite loss. Since the swap costs a full
  re-ingest of every stored vector plus a re-baseline, a tie does not pay
  for it. Chat/reason/answer/distill were already unified on `granite4:3b`;
  the judge stays a different family on purpose (an instrument must not
  grade its own answerer). Tradeoff, named: 300 probes with 13 discordant
  pairs only resolves large gaps — a real ±2-point difference could still
  hide, so this decision is "no evidence to move", not "proven equal".
  Caveat recorded for anyone reading the raw numbers: `--max-qa 30` takes
  the *first* 30 QA per dialogue, which skews the category mix (122
  multi-hop, 131 temporal, 5 single-hop, 0 adversarial), so 0.060 is NOT
  comparable to the 0.137 full-benchmark baseline.
  Decided by: verify-before-claiming, name-the-tradeoff. Supersedes: the assumption
  that model unification is free.

- 2026-07-20 — Eval harness speed/precision pass. (a) Distilled-claims disk
  cache (`eval/cache/`, keyed on prompt+model+seed, `--fresh-distill`
  bypass): re-runs skip the distill phase and ablation modes compare over
  byte-identical graphs — paired comparison needs fewer seeds. (b)
  `--concurrency N`: probe and judge phases run as Semaphore-capped tokio
  tasks with index-ordered aggregation (deterministic reports; default 1 =
  serial, baseline-identical). (c) `constants::MIN_DELIVER_SCORE` (0.40)
  and `MAX_DELIVER_RESULTS` were dead code — the shipped
  `RetrievalConfig::default` never gated delivery (0.0), so the
  improvement plan's "already gates delivery" claim was false; constants
  deleted, plan corrected, `--min-deliver` flag added so the abstention
  floor sweep (0/0.2/0.4) is runnable. (d) `--probe-log` JSONL (question,
  gold, pred, verdict, abstained, top_cosine per probe) — the artifact
  judge calibration and coverage-bar calibration both need. (e)
  Embed/answer/judge transport failures are counted and printed instead of
  silently shrinking denominators. Decided by: verify-before-claiming (the
  dead-constant catch, error accounting), delete-superseded (the two dead
  constants), name-the-tradeoff (concurrency>1 trades latency fidelity and
  VRAM for wall clock; cache trades disk for repeatability). Supersedes:
  serial-only eval, uncounted probe drops, the dead deliver constants.

- 2026-07-20 — The eval ablation formerly named "oracle" is renamed
  **grounded** (`--context-mode grounded|grounded-retrieval`, code + docs).
  "Oracle" is the standard test-oracle term but collides with this repo's
  `ORACLE.md` governance file and confused a reader; repo-local clarity wins
  over literature convention. Decided by: name-the-tradeoff (loses the
  standard term, gains an unambiguous name). Supersedes: the oracle naming in
  the entry below.

- 2026-07-20 — LoCoMo improvement plan items 0–5 implemented (measurement
  first, fixes where the plan called them mechanical). (a) Loss attribution:
  `locomo_eval --context-mode kern|grounded|grounded-retrieval` — grounded
  answers from the full conversation at 32 k ctx (rendered dialogues measure
  11–24 k tokens; the 8 k default and a first-guess 16 k both truncated —
  caught because the smoke run abstained on early-session facts),
  grounded-retrieval answers from the top-10 claims nearest the gold embedding
  and records the `gold_nearest_cosine` distill-coverage distribution
  (item 5 rides the same run). (b) Abstention seeded in the product path:
  `answer_prompt_from` instructs the exact `NO_ANSWER` string, empty-context
  synthesis returns it without an LLM call, and a unit test pins both to
  `locomo::is_abstention`'s marker set. (c) Distill prompt resolves relative
  dates against the session-date header; `valid_from` deliberately not
  requested — the eval worker path drops it. (d) Short-answer shape is
  eval-only via the new `QueryOptions::answer_style` (product prompt
  untouched). (e) Multi-hop: the plan's "expansion is one hop deep" claim
  was WRONG — `expand()` is a beam search and always was in this tree; the
  doc is corrected, and `--multihop-paths` now measures the real question
  (are gold-supporting claims graph-connected within 2 hops?) before any
  fix is chosen. Supporting: `LlmClient::with_num_ctx` builder.
  Decided by: avoided-question-first (attribution before fixes), verify-before-claiming
  (the one-hop correction, the truncation catch), name-the-tradeoff
  (32 k ctx slower but measures the ceiling, not recency). Supersedes: the
  plan's unimplemented status and its one-hop expansion claim.

- 2026-07-20 — `docs/kern/locomo-improvements.md`: the improvement plan the
  baseline funds, ranked by leverage. Leads with the loss decomposition
  (grounded-context / grounded-retrieval / baseline ablations) because every
  downstream fix guesses differently about where the 0.86 headroom is lost;
  then abstention seeding (prompt never asks for it, `answer_bench` proved
  granite can), multi-hop (expansion verified one-hop in
  `retrieval/expand.rs`; ingest links only Similarity+Provenance), temporal
  date resolution at distill, answer-shape F1 handicap, distill coverage,
  judge calibration. Decided by: avoided-question-first (the decomposition
  before the fixes). Supersedes: nothing — first plan against a measured
  number.

- 2026-07-20 — The LoCoMo baseline is recorded: full locomo10 (1986 QA),
  seeds 0/1/2, default local models (granite4:3b answer+distill,
  qwen2.5:7b judge at temperature 0). **Overall judge+abstain
  0.137 ± 0.018**; per-category table and per-seed numbers in
  `docs/kern/locomo-baseline-2026-07-19.json` +
  `docs/kern/eval-locomo.md`. p50 full-pipeline latency 901 ms. Roadmap
  question 1 ("what is the baseline?") is answered and replaced by the two
  craters the measurement exposed: multi-hop 0.042 ± 0.011 and adversarial
  abstention 0.112 ± 0.103; HyDE-gating and RRF-merge questions unblock.
  The number is far below the Zep/Mem0-class ~0.6+ the north star names —
  now measured, not assumed. Decided by: verify-before-claiming.
  Supersedes: the "validated but no baseline" status of 2026-07-16 and
  judging retrieval changes against intuition.

- 2026-07-19 — Gravitons replace the single per-kern "purpose". The anchor
  concept is renamed graviton end to end (~280 sites: types, routing, MCP
  tool, CLI, gossip, digest, docs) and grows into multi-focus attractors:
  `Kern.mass` (default 1.0) makes a graviton pull harder — ingest routes by
  `cosine_distance / mass` (1e-6 floor, both child selection and retain),
  and a new query-time pass (`retrieval/gravity.rs`) adds
  `gravity_weight (0.15) * max_over_gravitons(mass * max(0, cos))` to
  ranking (max, not sum; 0 disables). Seed text may be a full
  document/message, embedded whole. Dead `purpose` fields deleted from
  `wire.rs`. Tradeoff, named: gossip JSON field rename
  (`anchor_*` → `graviton_*`) breaks pre-rename federation peers — accepted,
  federation is opt-in LAN and pre-1.0. Bench (workload trace, 3-run
  medians): recall@10/NDCG@10 unchanged with gravity on or off, gravity
  pass costs ~+7% p50 with 5 gravitons, zero with none.
  Decided by: delete-superseded, name-the-tradeoff, verify-before-claiming.
  Supersedes: the one-purpose-per-kern anchor model.

- 2026-07-19 — Kern rows bump to `FORMAT_V3`; the persist comment claiming
  appended fields "use #[serde(default)]" lied for bincode — positional
  decode never fills defaults on missing trailing bytes
  (`UnexpectedEnd`), so any appended `Kern` field silently broke every
  existing graph. Root fix, not a patch at one call site: `KernPreMass`
  legacy mirror decodes V1/V2 LMDB rows and unversioned `.kern` file
  shards (try-current-then-fallback), compat test proves a pre-mass shard
  loads with `mass = 1.0`. Decided by: fix-the-root. Supersedes: the lying
  serde(default) comment and V2-only decode.

- 2026-07-19 — The 2026-07-17 model consolidation is now actually in the
  code: `DEFAULT_REASON_MODEL` was still `qwen2.5:7b` and
  `DEFAULT_ANSWER_MODEL` still `qwen3.5:4b` in `src/config/` — the decision
  was recorded but never landed (`git log -S granite4 -- src/config` is
  empty). `reason.rs` now says `granite4:3b`; `answer.rs` aliases it.
  Decided by: verify-before-claiming. Supersedes: the drifted qwen defaults.

- 2026-07-19 — `strip_think` in `src/llm.rs`: reasoning models (measured
  with `glm-5.2:cloud`) leak chain-of-thought into `content` even with
  `think:false`, poisoning answers with `</think>`-delimited reasoning.
  All four non-stream content extraction points now keep only the text
  after the last `</think>` and drop any unclosed `<think>` tail; unit
  test covers the leak shapes. Streaming path unstripped — a stateful
  filter isn't worth it until a streaming consumer feeds stored text.
  Decided by: fix-bugs-on-sight. Supersedes: raw content pass-through.

- 2026-07-19 — `locomo_eval` gains `--answer-url` / `--judge-url` per-leg
  overrides (default `--url`), matching the per-leg routing kern's own
  config already has — an eval can now mix an Ollama embedder with a
  vLLM `/v1` answerer or a cloud judge. Also `KERN_EVAL_DEBUG=1` prints
  gold vs pred per probe. Decided by: builtin-before-built (the config
  layer already splits legs; the harness just never exposed it).
  Supersedes: single-URL eval wiring.

- 2026-07-19 — `VISION.md` absorbs `docs/vision.md`: the four autonomous
  properties (self-learning, structured, self-compacting, self-distributing)
  and the design principles land as failable criteria — graph-not-bag with
  content-hash ids, bi-temporal supersede, retrieval-learns-from-use,
  fail-open, opt-in coordinator-free federation. Corrected
  `docs/vision.md`'s stale north star (beat-a-vector-DB) to the
  agent-memory framing `docs/aspiration.md` already decided; removed stray
  markup at its tail. Decided by: delete-superseded. Supersedes: the
  vector-DB north star in `docs/vision.md` and the criteria-only
  `VISION.md`.

- 2026-07-19 — Removed the Claude Code plugin. Deleted `.claude-plugin/`
  (plugin + marketplace manifests, which referenced a `hooks/` dir that was
  never shipped). Genericized the ingest source scheme (`claude:{stem}` →
  `session:{stem}`, `claude://` → `session://`) in `src/ingest/intake.rs` and
  the cwd-relative contract comment in `src/config/capture.rs`. Reframed the
  README, FEATURES, SPECIALISTS, and docs to present kern as an agent-agnostic
  MCP memory daemon (capture = `.txt` deltas in `.kern/capture/`, recall =
  `.kern/digest.md` + the `query` MCP tool) with no client-specific plugin or
  hooks. Decided by: delete-superseded. Supersedes: the Claude Code plugin
  packaging.

- 2026-07-18 — Logging actually emits now: `main.rs` initialized a bare
  `tracing_subscriber::registry()` with no layers, so every event — including
  the flush-refusal warnings that would have exposed the persistence bug —
  was dropped. Replaced with an stderr fmt subscriber honoring `RUST_LOG`
  (default `warn`); stderr because `kern mcp --mcp-stdio` owns stdout for
  JSON-RPC. Decided by: fix-bugs-on-sight. Supersedes: the layerless registry.

- 2026-07-18 — A refused stale flush now absorbs the disk graph into the live
  one and retries, instead of replacing the live graph with the disk copy.
  The old path silently dropped every unflushed in-memory row whenever an
  external writer (CLI `kern ingest`) bumped the store epoch — the daemon
  held entities in RAM forever while LMDB stayed empty. New
  `merge::absorb_graph` reuses the gossip CRDT joins (`merge_remote_entity`,
  `merge_reason`) so both writers' rows survive; `save_graph_guarded` adopts
  the disk epoch and retries up to 5 rounds. Tradeoff: rows deleted by an
  external writer between two daemon flushes can resurrect from the daemon's
  copy — accepted, losing data silently is worse and GC re-deletes.
  Decided by: fix-the-root. Supersedes: the reload-and-drop refusal path.

- 2026-07-17 — Implemented Phase 1 of the federation integration plan
  (`docs/federation-integration-plan.md`): the correctness core. Added
  `OrSet` and `LwwRegister` CRDT primitives to `src/crdt.rs`. Added Lamport
  clock (`AtomicU64`) to `GraphGnn` with `bump_lamport`/`observe_lamport`.
  Extended `CrdtDeltaPayload` with `lamport`, `producer`, `lww_value`,
  `orset_delta` fields (`#[serde(default)]` for backward compat) and new
  `CrdtTarget` variants (`ReasonScore`, `ValidUntil`, `Statements`).
  `merge_entity` now unions `statements` (no more lost concurrent adds) and
  uses LWW for `valid_until` instead of wall-clock `join_min_time`.
  `merge_reason` uses LWW with `(lamport, producer)` tiebreak instead of
  max-join for `Reason.score` (fixes the critical bug: `degrade` lowers scores,
  max-join irreversibly lost the lowering on sync). Added shadow LWW fields to
  `Entity` and `Reason` with `#[serde(default)]`. Write sites (`refine_edges`,
  `degrade_entity_reasons`, `place_document`, `place_chunks`) stamp
  `(lamport, producer)` via `g.bump_lamport()`/`g.network_id`. Added
  `PendingDelta` queue to `GraphGnn` with `push_delta`/`drain_pending_deltas`;
  `commit_access_ids_with_half_life` pushes counter deltas. Added
  `start_delta_flush` heartbeat loop that drains and broadcasts. Wired Delta
  sender (counter increments), Pulse sender (maintenance tick + `tool_pulse`),
  and Question sender (shared-slot `BroadcastQuestionFunc` bridging
  `registry.open` → `start_gossip` ordering). `handle_crdt_delta` handles all
  new `CrdtTarget` variants and observes incoming Lamport. 736 tests pass,
  fmt clean, build green with `--features bench`.
  Decided by: verify-before-claiming. Supersedes: the audit entry above.

- 2026-07-17 — Audited the federation roadmap (F0–F4) against the codebase
  at v1.0.0 and wrote `docs/federation-integration-plan.md`. Every roadmap
  claim verified against source: Delta/Question/Pulse are receive-only (no
  sender anywhere in `src/`), `Fetch` is single-thought only (no `AntiEntropy`
  variant), `crdt.rs` ships only GCounter/PnCounter (no OR-Set/LWW-Register),
  `merge_entity` never unions `statements`, `valid_until` is wall-clock LWW,
  transport is raw TCP with cleartext UDP `network_id`. One correction: the
  roadmap says `Reason.score` has "no merge rule" — `merge_reason` does a
  max-join; the real bug is that max-join is wrong for a non-monotonic field
  (`degrade_entity_reasons` lowers scores, max-join irreversibly loses the
  lowering on sync). Integration plan: Phase 1 (Lamport clock + delta/pulse/
  question senders + OR-Set for statements + LWW-Register for score/valid_
  until), Phase 2 (`AntiEntropy` bulk pull on rejoin), Phase 3 (mTLS +
  payload signatures + `network_id` as secret), Phase 4 (per-peer rate limit +
  divergence metric + remote heat floor). Refined ROADMAP item 4 into four
  specific gating decisions.
  Decided by: verify-before-claiming. Supersedes: nothing.

- 2026-07-17 — Strict comment sweep across the whole crate: doc comments
  (`///`/`//!`) and rationale prose are now in splinter notes, not source.
  Descriptive docs, derivations, benchmark provenance, and restatement were
  moved into per-file `.splinter.md` notes (the durable node memory) before
  deletion; only load-bearing hazards a maintainer would trip over — SAFETY
  blocks, lock ordering, must-run-before constraints, LMDB single-open,
  data-loss/crash windows, wire-format byte layout, units, platform-quirk
  workarounds — stay inline (tightened to ≤2 lines; SAFETY verbatim). Whole
  crate: 2324 → 625 comment lines; `///`/`//!` 1598 → 18. 154 source files,
  123 notes. Restored clap `///` help text on `bin/retrieval_bench` and
  `bin/locomo_eval` after confirming its deletion emptied `--help` output.
  Supersedes the softer first pass (594fb5d), which only thinned inline
  prose and left the doc blocks. Build green across the workspace
  (`--all-targets --features bench`), fmt clean, 723-test suite passing.
  Decided by: comments-last-resort. Supersedes: 594fb5d.

- 2026-07-17 — `start_entity_sync` (gossip handler) and `resource_thoughts`
  (MCP resources) had the same non-deterministic
  `partial_cmp.unwrap_or(Equal)` sort without id tiebreaks. Entity sync
  truncates to 32 entities — which entities get federated varied on heat
  ties; resource thoughts truncates to TOP_THOUGHTS — which thoughts appear
  in the listing varied on score ties. Both now use `cmp_rank` with entity
  id. Added per-scope and per-function ratings as splinter notes on
  `src/gossip/handler.rs` and `src/mcp/resources.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `build_digest` and `build_connections` (the digest generator
  that writes `.kern/digest.md` injected into every session by the
  `SessionStart` hook) sorted by `partial_cmp.unwrap_or(Equal)` with no id
  tiebreak, so equal-heat×confidence ties broke non-deterministically — the
  same graph could produce a different digest across runs. Both now use
  `cmp_rank` with entity/reason id tiebreaks, making the digest reproducible.
  Added per-scope and per-function ratings as a splinter note on
  `src/retrieval/digest.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `merge_seeds` (softmax seed merge) had the same
  non-deterministic `partial_cmp.unwrap_or(Equal)` sort as the two seed
  functions fixed in the prior commit. Now uses `cmp_rank` for a
  score-desc/id-asc total order, consistent with the rest of the seed path.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `seed_important` and `seed_by_reason` sorted by
  `partial_cmp(...).unwrap_or(Equal)` with no id tiebreak, so equal-cosine
  ties broke non-deterministically (parallel iteration order) — the same
  class of bug fixed for HNSW in `af8724d`. Both now use
  `crate::base::util::cmp_rank` (score desc, id asc), consistent with
  `fuse::rrf`, `search::merge_hits`, `lexical::search_filtered`,
  `store::cold_search`, and `vector_backend::union_rank`. The seed list order
  feeds `truncate(seed_k)`, so deterministic tie-breaking makes which
  entities survive the seed cut reproducible across runs on the same graph.
  Added per-scope and per-function ratings as a splinter note on
  `src/retrieval/seed.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `Config::load` fallback path doubled the `kern/` segment when
  `dirs::config_dir()` returned `None`: the chain
  `.unwrap_or_else(|| cwd.join(".kern")).join("kern").join("kern.toml")`
  produced `cwd/.kern/kern/kern.toml` instead of `cwd/.kern/kern.toml`.
  Restructured to `.map(|d| d.join("kern").join("kern.toml")).unwrap_or_else(||
  cwd.join(".kern").join("kern.toml"))` so the `None` fallback hits the
  project-local path, matching the intent. Latent — `dirs::config_dir()`
  returns `Some` on all supported platforms (Linux/macOS/Windows) — but the
  fallback was wrong. Added per-scope and per-function ratings as a splinter
  note on `src/config/mod.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `is_local_ollama` matched `localhost` and `127.0.0.1` as bare
  substrings, so a URL like `http://notlocalhost.com` false-positive-matched
  and would have been routed to Ollama-native `/api/*` calls a non-Ollama host
  404s on. Tightened to `//localhost` and `//127.0.0.1`, anchoring the host
  check to the URL authority component (after the `http(s)://` prefix); the
  `:11434` port marker stays loose as the WSL-gateway heuristic. New test:
  `notlocalhost.com` is NOT local. Added per-scope and per-function ratings
  as a splinter note on `src/llm.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `gossip/seen.rs` first reclaim loop used a bare `.unwrap()`
  on `VecDeque::pop_front` where every sibling invariant-guarded unwrap in
  the tree uses `.expect("…checked above")`; the second loop right below it
  already used the `let Some(…) = else { break }` form. Replaced with
  `.expect("front checked non-empty above")` for consistency — same
  invariant (the `front().is_some_and(…)` guard above proves non-empty),
  now with a diagnostic message that survives a panic. Added per-scope and
  per-function ratings as a splinter note on `src/gossip/seen.rs`.
  Decided by: fix-bugs-on-sight. Supersedes: nothing.

- 2026-07-17 — `kern ingest` could never hold more than ONE retrievable
  thought: every CLI ingest silently superseded the previous one. Root cause is
  a one-word conflation present since the initial commit —
  `Source::Inline.hash` is an OBJECT ID (the MCP tool feeds its `object_id`
  into it), but `cmd_ingest` passed `"user"`, the USER_SOURCE *trust* string
  copied off the `clamp_confidence` call on the line above it. Every CLI ingest
  therefore hashed to the SAME external id, and `accept()` supersedes any
  entity sharing one: each new thought invalidated its predecessor and evicted
  it from the ANN indices, leaving it in `kern.entities` (so `health` still
  counted it) but unreachable from `query`/`search`. Two arbitrary `kern
  ingest` runs are not revisions of one object, so the fix is no identity at
  all — empty `hash` -> `source_id()` is `None` -> no supersede — which is
  exactly what the MCP path already did with `object_id` unset. Found by
  actually reading a graph instead of trusting `status=committed`: 3 CLI
  ingests reported committed, `health` said `thoughts: 3`, and `search` returned
  1. The tell had been on screen three times earlier in the session as a
  "superseded by a newer version" chain between plainly unrelated facts, and was
  read past each time. Proven: both regression tests fail on `hash: "user"`;
  after the fix 3 unrelated CLI ingests each rank #1 for their own query
  (0.65-0.73 vs ~0.3 for the others) and carry Similarity edges only, no
  Supersedes. Scope: CLI only — the MCP ingest tool was never affected, and
  passing a real `object_id` still supersedes, which is the intended update
  semantics. Deduping identical text is unaffected: that is vector dedup, a
  separate mechanism. Tradeoff: `kern ingest` now has no way to express "this
  revises that" — correct, since it never had a coherent way to say WHICH
  object, and the MCP tool's `object_id` is the honest place for
  it. Decided by: fix-the-root, fix-bugs-on-sight, verify-before-claiming.
  Supersedes: the `hash: "user"` inline source and any belief that a
  `status=committed` ingest implies a retrievable thought.

- 2026-07-17 — kern was doing NOTHING on WSL, silently, for weeks — found while
  installing the new build, and fixed at the root. Evidence, not inference: 13
  daemons on this machine, uptime since Jul 14, every one of them `thoughts: 0`.
  Root cause is the zero-config promise colliding with WSL2 NAT networking —
  Ollama runs as a Windows host process, kern's loopback default
  (`http://localhost:11434`) resolves inside the WSL VM where nothing listens,
  so every embed returned a transient connect error and ingest re-spooled the
  job forever. Nothing crashed and nothing surfaced: the failure mode is an
  empty graph. New `config::wsl` repoints loopback LLM endpoints (embed /
  reason / answer) at the default-route gateway, but ONLY when all of: running
  under WSL, the URL is loopback, loopback is dead, and the gateway is live —
  probing loopback FIRST so mirrored-mode WSL2 and an in-distro Ollama keep
  their loopback and pay no rewrite. An explicitly configured URL is never
  second-guessed. Proven by controlled experiment in one scratch dir with no
  config file: new binary `status=committed` (`thoughts: 1`), old binary
  `status=failed`; then end-to-end on a real project through a live daemon
  (`thoughts: 2, reasons: 2`) with granite4:3b resident at 100% GPU. Tradeoff:
  `Config::load` now costs up to two 300 ms TCP probes on a WSL box whose
  loopback is dead — paid once at startup, only on the default URL, and only on
  the platform that would otherwise fail 100% of the time; non-WSL machines
  exit on the `/proc/version` check before touching the network. Gateway comes
  from `/proc/net/route` rather than `/etc/resolv.conf`'s nameserver, which
  diverges under `generateResolvConf=false` or custom
  DNS. Decided by: fix-the-root, verify-before-claiming, name-the-tradeoff.
  Supersedes: the assumption that a loopback default is portable, and the
  vLLM doc's WSL note as the only place this hazard was written down.

- 2026-07-17 — A stock install is now two `ollama pull`s and no config file:
  `DEFAULT_ANSWER_MODEL` aliases `DEFAULT_REASON_MODEL` (granite4:3b), so ONE
  llm runner serves both LLM legs beside a separate embedder. The consolidation
  paid for itself by dissolving the `num_gpu:0` reason pin rather than by
  saving VRAM. Root cause found by measurement, not reading: Ollama does NOT
  start a second runner when the same model tag arrives with a different
  `num_gpu` — the first placement wins and later calls silently reuse it, so an
  unconditional pin would have stranded the shared runner on the CPU and made
  every `/ask` pay CPU inference. But the pin only ever existed to stop a
  *distinct, larger* reason model evicting the answerer from an 8 GB card, and
  one model cannot evict itself — so `Client::pins_reason_to_cpu` now pins only
  when reason and answer resolve to different models or endpoints. Net effect,
  verified end-to-end: stock kern loads granite4:3b at 100% GPU serving both
  distillation and `/ask`, where the identical call previously ran 100% CPU;
  ~2.9 GB llm + ~2.1 GB embedder fits an 8 GB card with headroom. Distillation
  moved off the CPU entirely — a far bigger win than the model shrink.
  Tradeoff, named: qwen3.5:4b answers modestly better than granite4:3b (more
  complete, and granite sometimes restates context despite the prompt forbidding
  it), so a small answer-polish dip buys the simpler install and the
  GPU-resident reason leg; `[answer] model` restores the split and re-arms the
  pin automatically. New `scripts/answer_bench.py` is the evidence (14 cases
  incl. multi-hop, distractor, superseded, negation): granite is content-correct
  on every case and 4/4 on declining when context lacks the fact — the leg's
  real failure mode. Its scored 8/10 vs qwen3.5's 10/10 OVERSTATES the gap; two
  "misses" were verified scorer false negatives (right answer, wrong phrasing
  vs the gold string), which is also why the first version of that bench was
  discarded: it saturated at 6/6 for both models and could not discriminate
  until the hard cases were added. Embedder left ALONE and stays a separate
  model: new `scripts/embed_bench.py` (retrieval recall, not similarity vibes)
  measures the current qwen3-embedding:0.6b at 94% recall@1 / 100% recall@3 /
  MRR 0.971 — already near ceiling, so "use a bigger embedder" was tested and
  REJECTED: embeddinggemma (768 dim) beats both 1024-dim candidates on every
  metric incl. separation margin (+0.192 vs +0.161), i.e. bigger is measurably
  not better here, and mxbai-embed-large (1024 dim) has the WORST margin.
  Switching the embedder default would force `kern reembed` over every existing
  store — a migration this saturated 17-query bench cannot
  justify. Decided by: verify-before-claiming, fix-the-root, name-the-tradeoff.
  Supersedes: the unconditional `num_gpu:0` pin, the `qwen3.5:4b` answer
  default, and the "OPPOSITE optimization targets" rationale for splitting
  [answer] from [reason] (the knobs stay; only the defaults merge).

- 2026-07-17 — `DEFAULT_REASON_MODEL` is now `granite4:3b` (was `qwen2.5:7b`),
  chosen by measurement rather than reputation. New bench
  (`scripts/distill_bench.py`) scores candidates on kern's OWN distill prompt —
  8 conversations, 13 gold facts, recall by embedding cosine, all served
  through Ollama at temperature 0 — because leaderboard rank does not measure
  the task kern actually runs. granite4:3b ties the old 7B default on recall
  (12/13 vs 11/13 at a 0.72 match threshold), emits ZERO over-extraction noise
  against the baseline's 3, and never failed to produce parseable JSON (8/8),
  at 2.1 GB instead of 4.7 GB. Rejected: llama3.2:3b (85%, noise 5, one parse
  failure), phi4-mini (77%, one parse failure), qwen3.5:4b (85%, noise 4).
  The win is bigger than VRAM suggests: serving pins reason to CPU
  (`num_gpu:0`), so the reason leg always pays CPU inference and a 3B is ~2x a
  7B there. The eval judge (`locomo_eval.rs --judge-model`) deliberately stays
  on qwen2.5:7b — the judge is the measurement instrument, and this bench says
  nothing about judging quality. Web research (constrained-decoding and
  extraction-specialist literature) corroborated but did not decide it:
  schema-constrained decoding is a validity fix, not a quality fix, so it
  cannot recover recall a smaller model loses; parameter count is a weak
  predictor of extraction quality in the 1.7B-32B range; and the tiny
  extraction specialists (GLiNER 205-440M, Triplex 3.8B) fail kern's task
  shape — GLiNER emits verbatim spans and cannot paraphrase a claim, Triplex
  emits SPO triples under a non-commercial license. Tradeoff: 13 gold facts is
  a small sample, so the one-fact recall edge is within noise — the honest
  claim is "matches the 7B", carried by the robust signals (noise=0, format
  8/8, stable across two match thresholds), not by the recall delta. Two
  measurement bugs were found and fixed while establishing this: the bench let
  a format failure skip a conversation BEFORE counting its gold facts
  (rewarding unparseable output with a free pass — llama3.2 first scored a
  phantom 100%), and cosine matching at a 0.62 threshold produced a verified
  false positive (an unrelated postgres-overhead claim matched a "revisit if
  sharding" gold fact at 0.655), so recall is an upper bound and rankings are
  only trusted when they survive both 0.62 and 0.72. Left alone: `kind` label
  accuracy is ~33% even for the 7B — kern's taxonomy has overlapping
  categories (decision/project, fact/code-fact), a prompt problem that a
  bigger model does not
  fix. Decided by: verify-before-claiming, name-the-tradeoff, record-the-decision.
  Supersedes: the `qwen2.5:7b` reason default and its "larger models are
  sharper" framing in `docs/book/src/guides/memory-bank.md`.

- 2026-07-17 — Fixed two defects surfaced by the comment sweep, where a
  comment's claim and the code disagreed. `run_learned_propagation` discarded
  `unmarshal_weights` errors with `let _ =`, so a corrupt or version-stale
  snapshot silently cold-started the GNN every tick with no operator signal —
  it now logs at error level and still falls open, because a bad snapshot must
  not kill the tick. `retrieval_bench --values` validated twice: a pre-parse
  emptiness check with a useful message, then a near-unreachable post-parse
  check with a terse one; the pre-check is gone and the single post-parse check
  carries the good message. Trimming before the empty-filter also fixes
  `--values '   '`, which used to fail with a bare `ParseFloatError`. Verified
  by running the binary: empty, whitespace-only, and comma-only input all
  report the real error, a bad number still fails to parse, and a valid sweep
  still runs. Decided by: fix-bugs-on-sight, fix-the-root.
  Supersedes: the swallowed weight-load error and the duplicated `--values`
  validation.

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

- 2026-07-17 — vLLM (any local OpenAI-compat server) is now configurable with
  the existing `[reason]/[answer]/[embed]` url/model/key fields — no new
  config keys. Root cause was routing, not config: `is_local_ollama` matched
  any localhost URL, so a local vLLM at `http://localhost:8000` was sent
  Ollama-native `/api/*` calls it 404s. An explicit `/v1` suffix on the
  configured URL now forces the OpenAI-compat path (`wants_native` in
  `llm.rs`); bare local URLs keep the native path with its `num_gpu:0` /
  `keep_alive` / `num_ctx` serving protections. Eval's `seed`/`temperature`
  pins are now forwarded on the compat path too, so determinism survives a
  vLLM backend. Tradeoff: URL-suffix convention over a new per-endpoint
  `provider` key — zero config surface added, but the `/v1` marker is
  implicit; documented on the config fields. Decided by:
  builtin-before-built, fix-the-root, name-the-tradeoff. Decided by: the
  pinned list's fix-bugs-on-sight for the mis-routing itself.

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
