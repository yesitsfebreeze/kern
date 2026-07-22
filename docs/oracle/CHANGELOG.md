# Changelog

<!-- docs-check: historical -->

- 2026-07-22 — item 81 closed: the `kern mcp` `ProxyServer` answers every
  method its handshake advertises. `ProxyServer::handle_method`
  (`src/commands/mcp_cmd.rs:368`) dispatches the four graphless methods —
  `resources/list`, `prompts/list`, `prompts/get`, `ping` — through
  `handle_graphless_method` (`src/mcp.rs:249`), the one function the standalone
  `Server::handle_method` (`:219`) also uses, so the proxy and standalone
  surfaces are one implementation, not two that agree. `resources/read` rides
  the existing `call_tool` passthrough as `RESOURCE_READ_TOOL` (`:243`) —
  encoded/decoded by `encode_resource_read` (`:277`) / `decode_resource_read`
  (`:288`) with the verdict's exact code carried in the text block, because
  `CallToolRes` carries only `content` and `isError` and a code that does not
  survive the hop would turn `unknown resource` into a generic `-32000`. It is
  transport, not a tool schema entry, so `tools/list` is unchanged. `ping` is
  the one that mattered most: clients use it for liveness, so `-32601` there read
  as a dead server on the path an agent actually gets (`cmd_mcp` reaches
  `run_proxy` whenever a daemon exists). Five Rust tests drive the production
  `serve_rw` loop over a real proxy bound to a real daemon on a scratch socket.
  Decided by: fix-the-root (one shared dispatcher, not a second copy that can
  drift), reuse (ride `call_tool` rather than a fifth `KernRpc` wire method).
  Supersedes: nothing — the item described the defect, now removed.

- 2026-07-22 — eval data dir moved from repo-root `/eval/` to `/tests/eval/`, so
  all benchmark artifacts (datasets, reports, cache) live under `tests/` beside
  the harness scripts in `tests/e2e/eval/`. `.gitignore` `/eval/` →
  `/tests/eval/`; `common.py` `DATA_DIR = REPO / "tests" / "eval"`; `datasets.py`
  and `justfile` doc text repointed. Datasets stay gitignored (CC BY-NC, never
  committed). No script-path change — the runners stay at `tests/e2e/eval/`.

- 2026-07-22 — ACL and user identity removed wholesale, user-directed. kern is
  a single-trust-domain store: the process boundary (socket `0600` +
  `mcp-token` + anti-squat checks) is the whole access model, and multi-caller
  scoping is the embedding consumer's job — one kern per trust domain, or a
  gate in front. Deleted: `Acl {scope, users, groups}` off `Entity`,
  `principals`/`scope` off the MCP `ingest`/`query` schemas, `acl_admits` out
  of `matches_filter`, `src/mcp/acl.rs` (the edge-endpoint verdict), the
  default-deny resources gating, the ACL-aware dedup/supersede/rephrase
  carries, `Worker::{enqueue,run}_with_acl` (collapsed back into
  `enqueue`/`run`), `DirectJob::acl`, and the declared-never-consulted
  `AuthReq::principal` (with `PRINCIPAL_CLI`/`_MCP`/`_HUB`). No store or wire
  compatibility kept, per the same direction. Grounds: the mechanism was
  structurally unenforceable (caller-asserted principals, same-uid callers
  indistinguishable, empty `principals` = read everything), so it was a
  cooperative filter charging an access-control-sized tax — three read-surface
  bypasses found after "done", a per-edge verdict that failed open on
  non-resident endpoints, gossip shipping scoped rows ungated. 884 lib tests
  green after removal. Recorded as ROADMAP item 18 (REMOVED); item 24 keeps
  its transport half; item 9's "wait for a provable principal" exit is closed
  and its route-or-stay-local decision is now owed; item 20's author-principal
  blocker and item 79's "thread a real auth identity" alternative retired.
  Decided by: fix-the-root (delete the unenforceable mechanism, not another
  gate), name-the-tradeoff (re-adding costs a store bump, a wire change, and
  re-litigating every read surface; multi-caller scoping is forever the
  consumer's). Supersedes: the 2026-07-22 item 18 edge-ACL entries below —
  the enforcement they record is gone with the feature.

- 2026-07-22 — First real LoCoMo-10 numbers, and the claim standard is amended
  to permit exactly what they are. Run: `just eval-locomo --embed-url
  http://172.27.176.1:11434` (Ollama 0.32.1 on the Windows host across the
  WSL2 gateway — reconciliation now documented in README's quickstart;
  `localhost:11434` resolves inside the VM where nothing listens),
  `qwen3-embedding:0.6b`, direct path, k=10, release binary, ~16 minutes
  wall. 1536 questions scored, 446 adversarial + 4 evidence-less excluded,
  272 session ingests all committed, 0 truncation collisions. Results
  (`eval/reports/locomo-20260722-081539.json`): recall_any@1 0.3092,
  recall_any@5 0.5983, recall_any@10 0.7129, recall_all@10 0.5710, MRR
  0.4427, NDCG@10 0.4602. Per category, any@5: temporal 0.6760, single-hop
  0.6088, multi-hop 0.5532, open-domain 0.3696 — open-domain is the weakest
  slice and goes to Tier 8 as a finding. Query latency p50 0.62s / p95 0.68s,
  cold-process CLI wall clock (spawn + graph load + embed round-trip to
  another VM + retrieve), not a serving-path figure. Context for the one
  comparable number: YourMemory publishes LoCoMo-10 Recall@5 0.59,
  vendor-run, hit-definition unstated — kern's any@5 0.5983 is parity with
  that claim and nothing stronger; Zep (75.14) and Mem0 (92.5) are LLM-judged
  end-to-end scores in a protocol family that does not reproduce and are not
  comparable. The report's `commit` field names the parent commit because the
  harness landed in the same change that records this entry. Claim standard
  amended in ROADMAP's North star: retrieval-only claims permitted when they
  name harness, dataset, embedder, and hit definition; end-to-end claims
  remain forbidden. LongMemEval-S still pending (item 103 stays open).
  Decided by: verify-before-claiming (the numbers exist before the sentence
  about them), name-the-tradeoff (cold-process latency labelled as such).
  Supersedes: the "no quality claim of any kind" standard, in force since the
  2026-07-20 deletion, narrowed — not lifted.

- 2026-07-22 — item 30 closed, last half: the in-process ingest RAM queue
  reports its depth. `Worker::queue_depth` (`src/ingest/worker.rs:181`) is
  derived, not maintained — `max_capacity - capacity` off the mpsc channel
  itself, so the gauge cannot drift from the queue it describes and a job the
  run loop holds in flight is not counted. Surfaced beside the existing
  `ingest_queue_refused` counter on all three surfaces: MCP health JSON reads
  the serving worker's channel live (`src/mcp.rs:149`), `trnsprt::HealthRes`
  carries it `#[serde(default)]` (`src/trnsprt/src/kern_rpc/dto.rs:70`) so an
  old daemon's payload reads 0 rather than erroring, and `kern health` prints
  `ingest: queue N` daemon-sourced only (`src/commands/admin.rs:201`) — no
  daemon, no line, because the CLI's own worker is idle by construction and a
  local read is structurally zero (item 100's rule, applied unchanged).
  Proved at both ends: a unit test parks five jobs behind an embed gated on a
  closed semaphore, reads exactly five, and watches the gauge fall when the
  gate opens (`src/ingest/worker.rs:593`); the RPC test asserts a full queue
  reports depth >= 1 where a hardcoded 0 would still compile
  (`src/rpc/kern_rpc_server.rs:291`); and
  `tests/e2e/test_health_surface.py` parks watcher jobs behind a stalled
  fake-LLM embed and reads the nonzero line over the socket from a live
  daemon, after asserting idle reads 0 and no daemon prints no line.
  Tradeoff, named: the depth is sampled at answer time, not streamed — a queue
  that fills and drains between two `kern health` calls shows nothing, and the
  refusal counter remains the durable trace of a bound actually hit.
  Decided by: fix-the-root (a gauge derived from the channel cannot go stale;
  a maintained counter can), verify-before-claiming (both ends tested against
  a stalled worker, not asserted). Supersedes: nothing — completes the item
  the 2026-07-21 bound and the 2026-07-22 durability/LLM entries narrowed.

- 2026-07-22 — The retrieval-only public-benchmark harness ships: LoCoMo-10 and
  LongMemEval-S scored as recall@k / MRR / NDCG against the datasets' own
  evidence labels, no LLM in ingest, retrieval, or scoring — the replacement
  the 2026-07-20 deletion entry promised, now item 103 (run it) and item 104
  (full-pipeline variant). `tests/e2e/eval/`: `datasets.py` fetches into the
  gitignored `/eval/`, `score.py` is pure rank arithmetic (recall any-hit AND
  all-hit — multi-evidence questions make bare "recall@k" ambiguous, so both
  are computed and any quoted number must say which), `run_locomo.py` and
  `run_longmemeval.py` drive the real binary through the e2e harness.
  The stated blocker — turn-level claim provenance — turned out to gate only
  the distillation path: the direct path stores verbatim Documents whose text
  round-trips through `kern query`, so one turn per chunk gives turn identity
  for free and the harness needed zero Rust changes. Sessions batch through
  one `ingest --file` call and ride the paragraph split, ~15x fewer process
  spawns than per-turn ingest. Truncation collisions (two turns sharing a
  120-char printed prefix) are counted and resolved through `kern get`.
  Category 5 (adversarial) is excluded and counted — it has no evidence turns
  to rank, and it is where the public Zep/Mem0 dispute lived.
  Layout, user-directed the same day: `e2e/` moved to `tests/e2e/`, the eval
  runners live inside it, and `scripts/` is dissolved (`docs_check.py` is now
  `tests/docs_check.py`); justfile, CI, and every live doc anchor repointed.
  New recipes: `eval-fetch`, `eval-locomo`, `eval-longmemeval` — user-run by
  design; CI runs only the scorer unit tests and a `--fake-llm` smoke whose
  report marks itself MEANINGLESS.
  The 2026-07-22 competitor survey that sized this (sources in the session,
  vendor-claimed unless noted): Zep's LoCoMo went 84 -> self-corrected 75.14
  -> 58.44 when Mem0 re-ran it; Mem0's 2026 algorithm claims LoCoMo 92.5 /
  LongMemEval 94.4 with no independent reproduction; Letta scored 74.0 on
  LoCoMo with a plain filesystem and published it as an indictment of the
  benchmark; independent audits (Penfield Labs via "The Benchmark Theatre":
  6.4% of LoCoMo questions corrupted, the standard judge accepting 62.8% of
  wrong-but-topical answers; arXiv 2605.24060: scoring-target choice alone
  flips rankings) mean NO LLM-judged vendor number reproduces. The only peer
  publishing in this harness's protocol family is YourMemory (LoCoMo-10
  Recall@5 0.59, LongMemEval-S Recall@5 0.894, vendor-run).
  Tradeoffs, named: direct path only — distillation quality is unmeasured
  until item 104; a real run's number is a property of kern + the pinned
  embedder (`qwen3-embedding:0.6b` default), while the fake-embedder e2e
  floors stay as the regression gate; retrieval-only numbers are not
  comparable to anyone's LLM-judged scores and every report header says so;
  session timestamps are not ingested, so temporal categories will read low;
  near-duplicate dedup can absorb a gold turn into a survivor, which scores
  as a miss — the product's behavior, reported not patched. No quality claim
  accompanies this entry: the harness exists, the numbers do not (item 103).
  Decided by: verify-before-claiming, name-the-tradeoff, fix-the-root (the
  provenance blocker was re-derived from the code, not inherited from the
  deletion entry's summary). Supersedes: nothing — the LLM-judged eval
  stays deleted; this is its promised replacement, built.

- 2026-07-22 — item 102 closed: the GNN re-embeds one corpus identically in every
  process. Four sources, not the two `e2e/test_gnn_recall.py` confessed —
  unseeded weight init and negative-edge sampling, plus `build_gnn_snapshot` and
  `apply_gnn_updates` both walking `HashMap`s, so the feature-matrix rows, the
  edge indices and the HNSW insertion order (hence its entry point) were
  per-process. The last two are item 29's defect verbatim, in a second place that
  sweep never reached.

  **The seed derives from the corpus, not the kern id.** The kern id was the
  recommendation and it fails the item's own title: `Kern::new_unnamed` folds
  `now_nanos` into the id, so the same facts in a fresh project draw a new seed —
  measured over four e2e runs at 0.9306 / 0.8889 / 0.9167 / 0.9306. Sorted node
  ids are content hashes; streaming them through SHA-256 gives one seed per
  corpus, reproducible in any process, still distinct per kern. A constant seed
  was also measured: it reproduced, and proved there is no fifth source, but it
  would hand every kern in a fleet the same initial weights.

  Floors recalibrated only after three consecutive identical runs, and written as
  `65 / 72` rather than `0.9028` — the literal fails, because 65/72 is 0.90277…
  The old figures in items 28 and 97 were draws from a distribution; this one is
  a value, and they are no longer comparable.
  Decided by: verify-before-claiming — a recall floor set under a stochastic path
  scored the draw, not the code, and its own recorded minimum had already been
  falsified by a run nobody re-ran.

- 2026-07-22 — item 100 closed: `kern health` prints the serving daemon's
  degradation counts instead of its own process's structural zeros. Eight
  numbers — the seven fail-open `AtomicU64` statics summed into `degraded:` and
  `evicted:`, a `Store` field every `Store::open` zeroes — were read out of the
  CLI's own process, which opens a store and then runs no search, no scoring, no
  tick, no ingest and no merge. The `if degraded > 0` branch was not
  *sometimes* wrong; it was unreachable, and `evicted:` was 0 by construction.
  The operator's first diagnostic was the one surface that could not report.

  **The daemon's value wins outright; the two are never merged.** A `max()` or a
  sum reads as the safe choice — neither source can hide a nonzero — and it is
  the wrong one. These counters are not two samples of one quantity. They are
  two different processes' records of what *they* degraded, and only one of them
  is serving the store the operator is asking about. Merging would let a CLI
  that dropped a query on a dimension mismatch print that drop as though the
  daemon had suffered it, and a health surface that can attribute a fault to the
  wrong process is a surface an operator has to second-guess — which is the
  whole failure being fixed, in a new place. So `degradation_lines` takes the
  daemon's eight whole when a daemon answers and the local eight whole when none
  does, and the test that carries the decision is the inverted one: local
  counters nonzero, daemon healthy, and the printed answer is the daemon's
  zeros. It is the only formatter test a `max()` implementation fails, which was
  confirmed by writing that `max()` and watching exactly that test red.

  **The e2e driver the item specified could not be driven, and the item was
  wrong about it rather than the code.** `ingest_queue_refused` was the obvious
  lever — flood the queue past `QUEUE_CAP` from the CLI and read the refusal
  back — but nothing on the intake path calls the refusing enqueue; the one
  producer that must not be refused awaits capacity instead. A CLI flood cannot
  move it, so the counter was replaced with `ingest_dropped_chunks`, which a
  routed `intake drain` moves in the daemon: the fake LLM refuses any embed
  carrying `FAIL_MARKER` with a permanent 400, the distilled claims fail to
  embed, and the drops are counted over there. Three claims, and the test
  asserts **exactly 3** rather than "nonzero" — a blinded CLI cannot hold that
  number and a constant in the format string cannot match it. It also reads
  `kern health` once *before* the drain and requires the line to be absent, so
  the surface is shown tracking daemon state rather than merely printing.

  The formatter is pure over its two arguments — no static reads, no store — for
  the reason item 92 recorded: a test that reads a process static passes under
  `nextest`'s fork-per-test and reds under `cargo test --locked`'s one process
  the moment another test increments it. `HealthStats` gained `Default` only so
  those tests can name the one or two counters they care about; nothing in the
  tree constructs one, `graph_health_stats` still builds every field explicitly,
  and the derive is reachable from test code alone.

  Decided by: fix-the-root — the surface was not stale, it was reading the wrong
  process, and a merge would have preserved that error while hiding it behind a
  number that is never zero.

- 2026-07-22 — item 101 closed: **every anchor the widened `docs-check` exposed
  now points where its sentence says.** The filing said 18; the list was 20, and
  each was adjudicated by opening the cited line, finding the thing the sentence
  describes, and reading the new target back.

  Sixteen were genuinely wrong. Four of those were item 52's hazard exactly as
  predicted — a bare `` `:NNN` `` binding to the nearest preceding path and
  meaning another file. `bind_unix`'s continuation bound to `src/commands.rs`
  and meant `src/trnsprt/src/typed/local.rs`; `Worker::submit`'s bound to
  `src/ingest/direct.rs` and meant `src/ingest/file_watcher.rs`, where **the
  line number was right and only the file was wrong** — the quietest failure in
  the set, because nothing about it looks stale; and item 75's two doc-only
  leads bound to `src/base/graph.rs` while meaning
  `docs/kern/diskann-disk-index.md`.

  Two more were not anchors at all but **quotations the checker read as
  citations** — prose naming a broken ref in single backticks, which makes a
  fresh copy of it. Item 101's own first draft did this, and so did the note
  recording item 30's dead `concepts/acceptance.mdx` citation. The repo already
  had the fix (a doubly-backticked span is an illustration and is blanked before
  scanning); it just was not used. A page describing a broken citation must
  display it, not make it.

  **Two were false positives, and both are floor artefacts.** The `gc` row cited
  `tool_gc` correctly and `README.md:399` pins the version correctly; in each the
  only distinguishing token is below the three-character floor (`gc`) or is
  digits the tokeniser drops (`1.1.0`). Neither was silenced with `anchor-ok` —
  **no acquittal was written this pass.** The `gc` anchor was widened two lines
  to reach the `reaped`/`before`/`after` binding the row describes, and the
  `README` sentence now names the anti-entropy pointer sharing that line. Both
  true of the target, and both leave the anchor checked instead of dark. So
  `--strict-anchors` faces a 2-in-20 false-positive rate on the widened
  checker's first real list, neither one a judgement error.

  Two further repoints came out of the reading rather than the nominations:
  `Entity::acl` and `start_gossip`'s builders were both cited into unrelated
  spans and neither was nominated, because a long enough citing block shares
  words with almost anything. The content check bites where the sentence is
  short; that is the residual, and it is worth knowing before the flag is armed.

  Decided by: verify-before-claiming — a repoint is a claim, so each one was
  read back at its new target before it was believed.

- 2026-07-22 — merged item 18's close, and filed item 101 for what the merge
  exposed. 229 + 1 + this one = 231.

  **A second empty merge, same cause as the first.** The reap left `MERGE_HEAD`
  set with a staged diff identical to HEAD — `git show :src/ingest/file_watcher.rs`
  had none of the branch's work while the branch had all of it — and a stale
  `.git/index.lock` sat beside it with no process holding it. Committing would
  have recorded item 18 as merged while discarding it. Caught by the same check
  as last time: the branch's test name was absent from the index.

  That check is now worth stating as a rule rather than a habit. **After any
  merge, confirm one symbol from the branch is present in the index.** A clean
  `git status`, a plausible `MERGE_HEAD` and an empty conflict list are all
  consistent with a merge that did nothing.

  **The widened `docs-check` immediately nominated 18 anchors** — it learned to
  resolve bare continuation refs, and every one of those was wrong before and
  invisible. Filed as item 101 rather than fixed inline: two different faults are
  mixed in (ordinary stale numbers, and item 52's bare-ref-continues-the-wrong-file
  hazard), and several need a judgement about which file a bare ref was meant to
  continue. Eighteen repoints at the tail of a merge is how one lands on a
  plausible wrong line and goes quiet again.

  The uncomfortable corollary: every "no anchors nominated" this run meant "none
  among the 63% the regex could parse". The checker's own improvement is what
  falsified its previous coverage claim — which is the same lesson as the four
  false-green gates, arriving from the opposite direction.

  Decided by: verify-before-claiming — a green checker measures what it looks at.

- 2026-07-22 — item 18 closed by deciding: **a watched file is public.**
  `Acl::default()` names nothing, `Acl::is_public` is exactly that emptiness, and
  both watcher legs pass it while `drain_direct_once` carries the payload's own
  ACL rather than stamping one.

  Tenant-default lost on the same ground item 20's `source_trust_user` did. There
  is no tenant identity on the wire — item 24's principal is declared, not
  proven, and its residue says same-uid callers are indistinguishable. Stamping
  `scope: "tenant"` names a boundary nothing can verify, which reads as
  enforcement and is not. Configurable lost because it ships a knob plus a
  default and asks the same question at the default.

  **The slice's own test was a false green, and the sixth of this run.** Its
  agent died mid-falsification — its last words were "now the drain-carrier
  mutation" — leaving a mutation in `Worker::submit` still applied in the tree.
  Committing that would have shipped the exact design the decision rejects: a
  watcher stamping `scope: "tenant"`. Restored, then falsified properly, and the
  test passed *with the mutation in place*: it drains only the durable leg, while
  `submit`'s job waits in a channel for a worker loop the fixture never spawns.
  Renamed to `a_watched_file_is_public_once_the_durable_leg_commits` and the gap
  written into the item.

  Two things worth keeping. **A stalled revert-check leaves the tree mutated**,
  and a mutation that compiles and passes is indistinguishable from intent —
  three agents have now died mid-verification, and this is the first where the
  leftover would have inverted the decision being made. And the false green was
  the trap this brief named in advance ("a test where no entity is scoped passes
  whatever the default is"); naming it did not prevent it, catching it required
  running the mutation.

  Decided by: name-the-tradeoff — public is the honest default when the system
  cannot verify the alternative, and the cost is that a watched file is readable
  by every caller.
- 2026-07-22 — item 30's distill leg got a failure channel, and its ceiling got
  chosen. `complete_func` ended `.and_then(Result::ok).unwrap_or_default()`, so a
  600 s timeout, a refused connection, an HTTP 500, an auth rejection and an
  empty completion all arrived as `""` — no log, no counter, six call sites.
  `is_transient` already classified exactly those cases and was consulted only on
  the embed leg. `LLM_TIMEOUT` becomes `[reason] timeout_secs`, default 600, so an
  unconfigured kern is unchanged.

  The defect was confessed in shipping strings before anyone filed it:
  `record_stuck` already wrote *"the reason model returned no parseable claims
  (prose reply, or endpoint unreachable)"* — naming both causes and conceding it
  could not say which, because by then there was nothing left to say it with.
  Decided by: fix-the-root — the item called this a tuning question about a
  `const`, and the tuning half was the smaller half; a bound nobody can observe
  being hit is not a bound.

- 2026-07-22 — item 93: the anchor checker had been reading 63% of the anchors
  and reporting on all of them. 223 + this one = 224.

  `scripts/docs_check.py` verifies that every line citation points at a line
  that exists, and since 2026-07-21 that the line still reads like the sentence
  citing it. Both of the previous passes on this item tuned the *content* rule —
  tokenising, stemming, a three-character floor, a precision/recall trade
  recorded twice — and neither asked the prior question: what does the scanner
  see at all? `REF` demands a literal `src/` prefix, so two forms it never
  matched were invisible. A bare continuation, where a bullet names
  `src/base/store.rs:624` once and then cites the next eight functions by line
  alone. And a bare `place.rs:112`. Together, **245 of 664 line anchors — 37% —
  had no existence check and no content check on them.**

  That is the loop's own instrument, and the blind spot sat exactly where the
  cost is: the second pass re-pointed 29 anchors and had to count two of them by
  hand, "the continuations the regex never sees", in its own words. It named the
  gap and did not read it as a gap.

  Both forms now resolve against the last file cited before them and the scope
  resets at each heading, because that is how a human resolves them. A bare name
  with no antecedent falls back to a unique match under `src/`; `types.rs` is
  four files, so an ambiguous one is reported rather than guessed — a checker
  that picks one at random is worse than one that says it cannot tell. A
  doubly-backticked span is a quotation of the form rather than a use of it, so
  an item discussing anchors can display one without citing it. References
  checked: **834 -> 1008.**

  **It found a dead reference on the first run, and its shape is the argument
  for the whole change.** `ROADMAP.md` cited `Drop for LocalListener` by line
  alone under a paragraph whose last named file was `client_local.rs`, which is
  146 lines long; the symbol is at `src/trnsprt/src/typed/local.rs:654`. The
  line number was right and the file was wrong. A continuation's existence is
  not a property of the anchor — it is a property of the anchor plus every
  citation above it, so inserting one unrelated reference silently re-points
  every continuation beneath it. Spelling the path out is the only fix that
  survives the next insertion, and that is what landed.

  18 nominations followed, adjudicated one at a time against the tree: **15
  true, 3 false — 83.3% precision on a population that had never been checked
  once.** Five are the same wrong-file class as the dead reference. They are
  reported and not fixed: every one is a `[surface]`, `[retrieval]`,
  `[lifecycle]` or `[federation]` claim and this pass owns `[process]`.

  The guard is a fixture page run through `check_page`, not an assertion about
  the regexes, and it was checked against the previous build rather than assumed
  to bite: there the page yields one visible citation and an empty failure list,
  here it yields five and fails the continuation that points past EOF.

  **Verified independently the same day, against the commit.** This item was
  reconciled, implemented and recorded in one pass, so the verify stage ran
  afterwards rather than alongside. `just docs-check`, `just check` and
  `just test` are green with both recall floors unmoved; the 834 -> 1008
  comparison reproduces against the prior script over the prior tree; and the
  four-loop collapse is verdict-identical — with the two new patterns neutered
  the sweep prints byte-for-byte what the prior build printed. Two corrections
  landed in the ROADMAP entry: the wrong-file class is four and not five, and
  `--strict-anchors` now exits 1 rather than 0, because the 18 nominations
  stand. Re-adjudicating them gives 12 true rather than 15 — the three in
  dispute are anchors a human resolves correctly and the checker does not, which
  is under-specification rather than rot, and that class is also the only one
  that can turn the run red. The open residual is that both new forms are read
  inside fenced code blocks, where a number that merely looks like a line can
  fail the run; none exist in the tree today.

  Decided by: fix-the-root — two passes had tuned the judgement of anchors the
  scanner could see, while a third of them were never handed to it.

  Decided by: verify-before-claiming — the precision number, the reference
  count, the refactor's neutrality and the exit codes were each re-derived
  rather than read off the commit message.

- 2026-07-22 — merged item 26, which closes it, and with it the last of the
  retrieval performance items. 225 + 1 + this one = 227.

  Item 26 took four passes and each one corrected the previous one's framing.
  Pass 1 was told the scores were graph-dependent and cacheable; they are
  query-personalised, so the item's prescription was a design the repo had
  already rejected. Pass 2 confined the iteration to the reached set and
  recorded a 1.4x regression at full reach. Pass 3 found the instrument that
  produced 1.4x was unfair to its own reference — 1.19x measured honestly — and
  added the near-N switch. Pass 4 found the remaining blocker was aimed at the
  wrong thing: the buffers cost for being *allocated*, not for being wide, so
  the sparse-vector-versus-bit-identity argument that had kept the item open was
  never load-bearing.

  Every correction came from measuring rather than from reading, and three of
  the four overturned something written down by the previous pass. That is the
  shape of an item that was hard because nobody had instrumented it, not because
  the work was hard: the final change is a thread-local and a zeroing loop.

  Worth recording against the temptation to treat a long-lived item as
  well-understood. Item 26 accumulated four passes of prose, and the prose got
  *more* confident while staying wrong about which term dominated. The
  measurements disagreed with the file every single time.

  Decided by: verify-before-claiming — four passes, four corrections, and the
  file was the thing being corrected each time.

- 2026-07-22 — item 26 closed: PageRank's four N-wide buffers are lent by the
  thread instead of built per query. **2,540,344 B → 40,344 B per call** at
  N=100k and 1.0% reach; largest single block 800,000 B → 16,384 B; flat in N at
  fixed reach where it used to be 25.40 B/node. Ranking is bit-identical and the
  existing gate proves it unchanged.

  **The item's own stated blocker was aimed at the wrong thing, and that is the
  finding.** It recorded that closing this needed a sparse rank vector, and that
  a `HashMap`'s iteration order would put the `+0.0` bit-identity argument back
  in play — a real cost, correctly feared, which is why the work sat. But the
  buffers do not cost anything for being *wide*; they cost for being *allocated*.
  Separate the two and the dense ascending vector — the thing the whole exactness
  argument rests on — survives untouched, and only the `calloc` goes. Nothing had
  to be re-argued because nothing arithmetic moved.

  Two things this cost, both said plainly rather than buried:

  - **The clock is smaller than the item promised.** 0.310–0.420 ms → 0.244–0.249
    ms at N=100k and 1.0% reach, four paired runs a side. The item's 0.18 ms
    "floor" was the whole per-query cost at that reach, not the allocation's
    share of it, and the share is ~0.065 ms. Direction consistent everywhere,
    magnitude small.
  - **The memory is resident now, not transient.** Each thread keeps 2.5 MB at
    N=100k for its lifetime, and readers run concurrently under the graph lock.
    Peak RSS is unchanged — the old path allocated the same per concurrent call —
    but the steady state grows with thread count.

  The gate is an allocator, not a stopwatch: `test_support::alloc_probe` counts
  bytes on the calling thread, and `a_narrow_query_allocates_nothing_sized_by_the_graph`
  runs one identical narrow walk against two graphs 4x apart in N and asserts the
  byte counts are *equal*, with no tolerance and no constant to tune. Reverted to
  per-call allocation it reports `413596 B against 113596 B for the same 244-node
  walk` — the 300,000 B being exactly 25 B × the 12,000 nodes of difference.
  Timing could not have carried this assertion: 2.5 MB of `calloc` is under the
  noise of this box.

  Decided by: fix-the-root — the root was the allocation, not the representation,
  and the representation was what the item had queued up to change.

- 2026-07-22 — item 28 closed: `gnn_train_refused` crosses the RPC and reaches
  `kern health`. Three edits — the field on `HealthRes`, the handler filling it
  from the same `tool_health` payload every other counter reads, and
  `tick_health_lines` folding it into the `degraded:` line. 980 → 982 tests.

  **Decided by: it folds into the existing `degraded:` line because it cannot
  join the other one.** `cmd_health` already prints a `degraded:` line for the
  seven fail-open counters, and an eighth fail-open counter obviously belongs
  there — except that line is built from `graph_health_stats`, which the CLI
  computes *in its own process*, and `TRAIN_REFUSED` is a global only the daemon
  ever moves. A CLI reading it locally sees 0 forever. The only counters a CLI
  can see at all are the ones that crossed the RPC inside `HealthRes`, and the
  tick line is the only `HealthRes`-derived degradation line there is. So the
  choice was never "new line or existing line" — it was "the line fed by the
  wrong process, or the one fed by the right one". Folding rather than adding
  also keeps `a_clean_daemon_prints_no_last_fault_lines` green unedited: a
  healthy tick still prints exactly two lines, so a quiet kern does not grow a
  third that always reads zero. That test staying green *unedited* is the
  signal — an edit to it would have meant the output shape changed rather than
  extended.

  **The verification found a real flake, and `just test` could not have.** The
  new RPC test spawns a real `Trainer` and blocks its runner, because
  `tool_health` reads the trainer's global directly and there is no seam to
  inject a payload through — a real refusal is the only way to make the counter
  nonzero. But `TRAIN_REFUSED` is one global per process, and CI runs `cargo
  test --workspace` where `just test` runs `cargo nextest`: one process for the
  whole suite versus one process per test. The new test refuses a full cap's
  worth of submissions; the trainer's own cap test asserts its delta is exactly
  1. **Measured: 5 red runs in 30 under `cargo test`, 0 in 40 under nextest.**
  Both tests now serialise on a test-only `REFUSAL_COUNTER`; 40 of 40 green
  after.

  The rule that leaves, and it generalises past this counter: **a test that
  moves a process-global must serialise against every test that measures one**,
  because a measurement is two reads and the gap between them belongs to
  whoever else is running. And the corollary about instruments — a green
  `just test` is not evidence about a global, because the runner that makes it
  green is the one that hides the defect. Two runners, two answers; the one CI
  uses is the one that counts.

  Also corrected in passing: `FEATURES.md` described the MCP `health` tool as
  carrying "the seven fail-open counters" and omitted this one, and its
  `src/tick/*` line-count had drifted ~660 lines behind the tree.

  Decided by: verify-before-claiming — the mutations were re-run, and the flake
  turned up only because the test was run under the runner CI actually uses
  rather than the one the justfile offers.

- 2026-07-22 — merged item 18's edge-ACL fix. 221 + 1 + this one = 223.

  The finding is worth separating from the fix. Item 18's *title* named a defect
  that did not exist — a bare `query {id}` does run `matches_filter`, and the
  existing withhold test proved it. The *body* enumerated four rendering surfaces
  and fixed two, leaving the other two named in a paragraph. **The real bypass
  was in the two it listed and skipped**, and a scoped Fact's text was reaching
  non-members through a public neighbour's edge for as long as that paragraph
  had been sitting there.

  So the item was simultaneously wrong at the top and right in the middle, and
  only reading the whole thing found the live leak. That is the third time this
  run a heading has pointed away from the actual defect — but the first where
  following the heading would have produced a *correct* conclusion ("already
  enforced") and closed the item over a real hole.

  The structural cause is in the fix: one endpoint verdict existed in
  `resources.rs` and the other three surfaces open-coded their own rendering.
  Four copies of one rule is why two were wrong, and neutering the now-shared
  `incident_edge` fails all three withhold tests together — including the
  pre-existing resources one, which is the evidence they share a verdict rather
  than agree by coincidence.

  Decided by: fix-the-root — an ACL enforced per surface is enforced wherever
  someone remembered, which is a different property from enforced.

- 2026-07-22 — item 18: the `query` tool gated the row and published its
  neighbours. Its title said "a bare `query {id}` still filters nothing", which
  had been a decision rather than a defect since 2026-07-21 — the id path runs
  `matches_filter`, and a bare read filtering nothing is the empty-principals
  default every single-agent caller depends on. Both are already pinned. Going
  looking for the defect the title no longer named found a real one: a `Reason`
  carries no ACL, `link` writes its body from up to 500 chars of **both**
  endpoint texts, and the row clearing `matches_filter` says nothing about its
  neighbour. `query {id: <public row>, principals: ["bob"]}` served an
  alice-scoped Fact's text verbatim through any public neighbour's edge; the
  ranked read did the same at 120 chars. `kern get` routes to the first of those.

  The gap was **written down and skipped**. The 2026-07-21 entry enumerated the
  four surfaces that render an entity's edges — `entity_detail`, the ranked
  `edges` array, `resource_thought`, `format_chains` — fixed the last two, and
  left the first two named in its own prose. A list of surfaces is not a fix for
  the ones on it.

  Fixed at the root: the endpoint verdict left `src/mcp/resources.rs` and became
  `src/mcp/acl.rs`, one `Endpoint` + `incident_edge` that all four renderings
  call. It takes the **admission rule** as a parameter rather than the
  principals, because the two surfaces disagree about what "allowed" means and
  have to keep disagreeing — resources can name no principal so its rule is
  `Acl::is_public`, while `query` takes the caller's. The `query` half is
  `acl_admits_entity`, the ACL predicate of `matches_filter` lifted out so the
  edge gate cannot re-derive the empty-principals default and get it wrong.
  Recall unmoved at 0.9306 / 0.9722 / 0.9471 — an ACL that changed an unscoped
  read would be the bug, not the feature.

  Title narrowed with it: the one decision still owed here is whether the file
  watcher gives `Document` entities a tenant-default ACL. Everything else the
  item still lists is item 24's residue, federation's, or a named later fix
  (storing the verdict on the edge at write time, which is what would make this
  gate cheap and fail-closed instead of per-read and fail-open).

  Decided by: fix-the-root — four copies of one verdict is why two of them were
  wrong, so the fix was to leave one.

- 2026-07-22 — the stale-heading defect has a second form, and three closed items
  were in it. Items 21, 94 and 97 led with the defect and appended "CLOSED":
  *"The e2e harness cannot exercise the GNN at all — closed"*, *"A near-duplicate's
  alternate wording is stored but indexed nowhere — CLOSED"*. Every word before
  the dash is false, and slice selection reads `^###` — a scan sees a live
  defect unless it reaches the end of a 120-character line.

  Aligned to the convention items 27, 95, 98 and 32 already use: **state the
  resolved condition, then the closure.** "A GC sweep pays one LMDB commit, not
  one per victim — closed". "The pre-auth frame is capped and deadlined —
  closed". The title reads true on its own, and the date is a footnote rather
  than the correction.

  This is the same failure as the seven titles retitled earlier this run, but it
  survived those passes because the earlier heuristic asked "does the title
  contain a closure marker" — and these do. The marker was present; the sentence
  was still false. A rule that checks for the presence of a word cannot see word
  order, which is the third heuristic this run to founder on the difference
  between form and meaning.

  Worth stating as a convention rather than three fixes, because the next closure
  will be written by whoever closes it: **an item's title is a claim about the
  repo, and it is read alone.** If it is only true when read to the end, it is
  not true.

  Decided by: fix-the-root — the recurring defect is titles written as edits to
  the old title rather than as fresh statements of the current truth.

- 2026-07-22 — merged item 97, and with it the last of the four verification
  gaps this run found in its own instruments. 218 + 1 + this one = 220.

  Worth collecting them, because they are the same defect wearing four faces and
  none was found by a failing test:

  - **The GNN recall gate ran no GNN** (item 97). `min_thoughts` 128 against a
    36-fact corpus, and the CLI has no tick loop at all — two independent
    reasons, and the second means the obvious fix would have changed nothing.
  - **The pre-auth cap test measured refusal, not allocation** (item 98). A
    refusal assertion passes while the daemon buffers 16 MiB first.
  - **The importance-scan test measured the helper, not the command** (item 25).
  - **The routing sublinearity assertion had no falsifier at all** (item 31) —
    its own generator collapsed the distinction it was asserting.

  Each was caught the same way: someone reverted the code and watched what the
  test did. Not by review, not by reading, not by any checker. That is now the
  single most productive habit in this loop, and it is worth stating why it
  works — **a test that has never been observed failing is an assertion about
  the world with no evidence behind it**, and the cost of getting that evidence
  is one revert.

  The corollary the four share: every one of them asserted the *outcome* rather
  than the *cost or the mechanism*. "Recall unchanged", "connection refused",
  "same seeds returned", "slope is sublinear" — all true statements, all
  satisfiable without the code under test running at all.

  Decided by: verify-before-claiming — four gates, four false greens, one method
  that found all of them.

- 2026-07-22 — item 97: `e2e/` now runs the GNN, and the gate refuses to score
  until it has. 38 → 39 e2e tests; `e2e` wall time **89.5s → 99.5s (+11%)**, all
  of it the new test's daemon boot and its 72 routed probes. Existing recall
  floors untouched and unmoved (0.9306 / 0.9722 / 0.9471).

  **The item's premise was right and its favoured closure is dead.** It blamed
  `DEFAULT_MIN_THOUGHTS=128` against a 36-fact corpus, which is true, and
  proposed growing the corpus past 128. Measured through the real binary with
  `do_gnn_propagate` temporarily instrumented: at 36 facts under a daemon the
  propagation is entered and returns (`entities=36 min=128`); **at 150 facts it
  still never runs**, because the boot cluster pass splits the root into 36 + 114
  and neither part reaches 128. `do_cluster` enqueues `GnnPropagate` only when it
  did structural work — and structural work is the same act that moves entities
  *out* of the kern about to be propagated. Growing the corpus fights that, at
  1.9s → 54.7s of CLI ingest for 36 → 150 facts, for a gate whose liveness would
  still depend on clustering behaviour nobody has pinned.

  There was a second reason nothing ran that the item did not have:
  **`test_recall.py` drives the CLI, and the CLI has no tick loop at all.**
  `do_gnn_propagate` is reachable only from `tick::start` — spawned by
  `store::Registry::open`, i.e. a daemon or `kern mcp` — and from `tick_sync`,
  whose one caller is a unit test. Lowering the threshold alone would have
  changed nothing there either.

  **So the shipped gate lowers `min_thoughts` in an e2e-only config *and* adds
  the liveness assertion that is the whole point.** A recall floor alone would
  have rebuilt the vacuous gate: green, about code that never executed. The
  propagation now logs `learned propagation applied` with a `nodes` count on
  success — failure was already loud, success was silent, and `gnn_vector` is
  dropped on persist, so nothing on disk could ever have answered "did it run".
  `e2e/test_gnn_recall.py` waits for that line, refuses a run covering under 30
  nodes, and only then scores.

  **Proved by breaking it.** With entity `i`'s propagated embedding written to
  entity `i+1` — the ranking bug a bit-identity proof cannot see — `cargo nextest
  run --workspace` was **972 passed** and `e2e/test_recall.py` passed printing
  its usual 0.9306 / 0.9722 / 0.9471, while the new test **failed 3 of 3**
  (recall@1 0.7917 / 0.7222 / 0.7361 against a 0.85 floor). Each new assertion
  was also reverted individually: restoring the shipped 128 gives "no propagation
  in 60s" with an empty daemon log, ingesting 6 facts gives "propagation covered
  6 nodes, under 30", and deleting the log line gives the 60s timeout again.

  **The floors are looser than the CLI corpus's, and that is a cost, not a
  choice.** Propagation seeds weights and samples negative edges from an unseeded
  RNG, so the number moves run to run: over **8 runs**, recall@1 **0.8889–0.9306**,
  recall@5 **0.9583–0.9722**, MRR **0.9219–0.9508**. Floors 0.85 / 0.93 / 0.88
  sit below the worst of that sample, roughly three probes of headroom — wide
  enough that a subtle ranking regression could hide inside it, and the broken-GNN
  runs above are the evidence that a real one does not.

  What this still does not measure: **production scale.** The gate propagates 36
  nodes, not 128+; `tests/gnn_scale.rs` is the only thing that runs at that size
  and it is `#[ignore]`d and asserts nothing about ranking. Recorded in item 97
  rather than left implied.

  Decided by: verify-before-claiming — the closure the item preferred was
  measured before it was built, and the measurement is what killed it.

- 2026-07-22 — the daemon stops standing down for a squatter, and stops
  unlinking a name it never checked. Item 24 residue 3, the bind half. 972 → 974
  unit tests; recall floors unmoved (0.9306 / 0.9722 / 0.9471, unretrieved 0).

  **The recorded bug was that `bind_kern_listener` believed whatever answered
  its `AddrInUse` probe.** A foreign socket accepted the connect, the arm read
  that as `AlreadyRunning`, and the real daemon exited. That half is closed by
  running the two checks `connect_kern` already ran — `require_owned_by_caller`
  on the name, the peer check on whoever answered — and returning a new
  `BindError::Untrusted` naming the foreign uid instead.

  **The unrecorded half was one line further down and is the wider one.** When
  the probe *failed*, the arm ran `remove_file` on a path nobody had verified.
  The sticky bit on `/tmp` makes that safe for a foreign socket, which is
  presumably why it read as safe — but a sticky bit protects a *target*, never a
  *link*. A symlink this uid owns pointing at somebody else's file is ours to
  unlink, so the old arm deleted it and bound a listener on a name a foreign
  path had been substituted into. Ordering the checks before the unlink closes
  that by construction: there is one `remove_file` in the function and it is
  past the `?`, so it cannot be reached on a name that has not been proved ours.
  The three error shapes were run rather than reasoned about — a dangling
  symlink refuses and survives, a vanished path refuses, a plain file this uid
  owns is still reclaimed.

  **The tradeoff, named: fail-closed costs a restart race.** A name that
  disappears between the `EADDRINUSE` and the stat — a predecessor exiting
  mid-race, whose `Drop` unlinks — now refuses where it used to rebind. A retry
  was the alternative and was not taken: in *this* arm the kernel has just said
  the name is held, so absence is a race, not the ordinary no-daemon case, and
  guessing wrong here is an unlink. The operator sees why, because the daemon's
  `Err` arm now `eprintln!`s as well as `tracing::error!`s — it previously only
  traced, so a refusal at the default level would have exited saying nothing,
  which is the same silent stand-down the change set out to remove.

  **The peer check in that arm was reported untestable. It was untested.** The
  claim was that catching it needs a socket bound by a second uid. Three ways
  around that were tried. A same-uid child with a different exe does not bite —
  measured, it is *correctly* accepted, because `SO_PEERCRED` proves a uid and
  not a program. An abstract-namespace socket and a `socketpair` do not bite —
  neither has a filesystem name, so a path bind never returns `EADDRINUSE` for
  one and the arm is never entered. Injecting the expected uid does bite, and it
  is the move this file already made once: `require_peer_uid` exists precisely
  because `require_peer_is_caller`'s refusal was otherwise unreachable. So the
  arm moved into `bind_unix(path, expected_peer)` and a test drives the whole
  arm — real listener, real `SO_PEERCRED` read — against a uid that is
  deliberately not the server's. Neutering the peer check now fails a test
  instead of nothing. `connect_kern`'s own call has no such seam and stays
  code-review-only: one line, not the two previously recorded.

  **Verified, not asserted.** Reverting only the arm's body to its pre-change
  form fails both new tests and leaves the other four bind tests green, so
  neither is a tautology. Reclaiming our own stale socket — the thing that would
  break every restart — was checked against a real second process rather than
  inferred from the mode tests: a same-uid, different-exe daemon bound the path,
  read as `AlreadyRunning` while alive, was `SIGKILL`ed, and the next bind
  reclaimed the leftover socket and hardened it to `0600`.

  Also corrected a live lie the change itself created — `SPECIALISTS.md` said
  the bind path checked neither credential, in the same commit that made it
  check both — and repointed the two relative anchors in item 76 that the
  five-line `commands.rs` edit shifted (`save_fn()` 892→897, `process::exit`
  954→959). Both are bare `:NNN` refs, which `docs_check.py` does not match, so
  they went stale silently while the checker stayed green at 823 references and
  zero nominations. No anchor was silenced; the anchor-ok count is unchanged at
  eight.

  Decided by: fix-the-root — the reported defect was the stand-down, but the
  same arm was unlinking an unverified name one line below it, and closing only
  the reported half would have left the wider hole with a green suite over it.
  And verify-before-claiming — "untestable without a second uid" was a claim
  about the world that turned out to be a claim about the code's shape, and the
  shape was ours to change.

- 2026-07-22 — merged item 98's pre-auth cap and deadline. 215 + 1 + this one =
  217.

  Worth recording what closing it revealed about the item I wrote. I filed 98
  from `FEATURES.md` §13's own words plus a read of the auth path, and explicitly
  told the slice I had tested neither half — the "huge allocation" shape might
  already be bounded by the transport codec. It was not, and the proof is that
  reverting the cap to 64 MiB fails the new test with *"the daemon buffered
  16777216 bytes from a peer that has proven nothing"*. So the item was right,
  but it was right on an untested reading, and the difference only became
  knowable when someone wrote the test that could have falsified it.

  The test's shape is the transferable part: it asserts on **bytes buffered**,
  not on the connection being refused. A refusal test passes while the daemon
  allocates a gigabyte first, which is precisely the defect. Three items this run
  shipped a gate that measured the wrong quantity and had to be rewritten — 25's
  eligibility, 28's recall, 31's fan-out — and each time the tell was the same:
  the assertion named the *outcome* rather than the *cost*.

  Also: the refusal path deliberately gives a timed-out peer nothing at all,
  while every other refusal answers. A reply to a silent peer is a free liveness
  probe that names the deadline, and all refusals share one message so the
  response cannot tell a caller how far it got. Neither was in my brief.

  Decided by: verify-before-claiming — an item written from a document is a
  hypothesis about code, even when it turns out to be true.

- 2026-07-22 — item 98: the pre-auth frame on kern.sock is bounded, 1 KiB and
  five seconds. Both halves were real; one was real by a mechanism the item did
  not name, and that difference decided the whole fix.

  **The item said "a frame declaring a huge length". There is no declared
  length.** `JsonEnvelopeCodec` is newline-delimited, so nothing on this wire
  announces a size — `FramedRead` reserves, reads and doubles its `BytesMut` for
  as long as `decode` returns `Ok(None)`, which is until a `\n` arrives. The
  allocation is not requested, it is accreted. That is worse in one specific way:
  a cap applied around `channel.recv()` would never fire, because on this input
  `recv()` does not return. The cap had to go into the decoder, where the buffer
  is, and had to measure an **incomplete** line — the only shape an endless frame
  ever has.

  **Measured before it was fixed, because the refusal was already there.** A peer
  writing 16 MiB with no newline was refused — at EOF, after the daemon had taken
  all 16777216 bytes into its buffer. Every "is it refused?" assertion was green
  through the entire defect. So the test asserts on bytes the daemon took, which
  is the buffer's size exactly, since nothing is consumed while `decode` returns
  `Ok(None)`.

  **The silent half was real as written**, but "occupies its accept slot" is not
  what happens — `serve_kern_rpc_loop` spawns per connection and keeps accepting.
  What is held is a task, an fd and a growing buffer, for a session item 24
  guarantees will never be authorised.

  **On timeout the connection closes without a word**, and this is the one place
  the path departs from "one message for every refusal". A peer that ran out the
  clock never spoke: there is no misconfigured client to inform, and a reply
  would be a free liveness probe that also names the deadline. An oversized frame
  gets the standard refusal instead, deliberately — it did speak, and a distinct
  answer would tell a caller which limit it hit. Tradeoff named: an operator
  whose client wedges mid-handshake sees a bare disconnect and must reach for
  logs. Worth it; a refusal frame is worth less to the honest case than it is to
  the dishonest one.

  Both limits are lifted the instant the frame is in hand, so `call_tool`'s whole
  documents keep the unbounded framing they need, and both are therefore strictly
  tighter than anything past the gate. Six new tests, workspace 978 passed / 16
  skipped; `e2e` recall unmoved at 0.9306 / 0.9722 / 0.9471.

  Decided by: verify-before-claiming — the item's stated mechanism did not exist,
  and the obvious assertion was green against the defect it was written for.

- 2026-07-22 — scanned every ROADMAP item for the recurring stale-heading defect
  and retitled two more: item 32 (body: "Closed 2026-07-21", title still naming
  the bias — and its own body says the fix the title implies would have made
  things worse) and item 18 (four bullets done, title still claiming it "gates
  everything else in this tier" when what is left is that a bare `query {id}`
  filters nothing). Sixth and seventh this run.

  The scan flagged twelve and I kept two. That ratio is the useful part: ten of
  the twelve already named their *remainder* correctly — item 26 "allocates four
  N-sized buffers", item 29 "was measured and refused", item 52 "a single-line
  seed still truncates". A crude "body says closed, title does not" rule cannot
  tell a stale title from a correctly-narrowed one, because both mention a defect
  that exists; only the body says which defect.

  So this stays a read, not a check. That is worth writing down after three
  attempts at automating docs reconciliation this run — the anchor nominator
  works because "does this line say what cites it" is decidable from two strings;
  gap-block coverage and stale headings are not, and both heuristics I tried
  produced mostly false positives. The lesson is not that automation failed, it
  is which questions are mechanisable: **content equality is; editorial accuracy
  is not.**

  Decided by: verify-before-claiming — twelve flags, two real, and the difference
  was only visible by opening each one.

- 2026-07-22 — merged item 30's durable watcher path. The interesting conflict
  was in `e2e/conftest.py`, and it is the first this run that needed a **code**
  merge rather than a pick: item 21 added `review_policy` to `write_config`,
  item 30 added `watcher_enabled`/`watcher_roots`, both to the same signature and
  the same emit block. Neither side was stale and neither was wrong — taking
  either would have silently dropped a parameter every future test in that
  branch's lineage depends on. Merged to carry all five, docstrings and emit
  blocks combined, verified by parsing the file rather than by eye.

  Worth naming because every previous conflict this run resolved to "take one
  side": newer text, or the branch that was younger than the fact. A signature
  two branches both extended is the case where picking is always wrong, and it
  is invisible in the diff — both versions look complete on their own.

  Also repointed three anchors the merge shifted (`effective_roots` 24→25,
  `wire_fetch` 1077→1093 twice), and took `cycle/2`'s substantive ROADMAP text
  over master's, which differed only in already-repointed line numbers.

  Decided by: verify-before-claiming — "both sides look complete" is exactly when
  a merge needs the union rather than a choice.

- 2026-07-22 — item 30's durable backstop for the file watcher, and the
  self-referential edge it opened. 934 → 939 unit tests, 34 → 36 e2e; recall
  floors unmoved (0.9306 / 0.9722 / 0.9471).

  **`DirectJob` gained a `source_tag`, and this is the load-bearing part of the
  change.** Routing the watcher through the durable intake means its records now
  travel a hop that was built for one producer: `drain_direct_once` named
  `AGENT_SOURCE` for everything it read, correctly, because everything there was
  minted by the MCP tool. Sent through unchanged, a watched file would come back
  labelled an agent assertion — undoing item 95's "the tag is the channel" and
  mislabelling the key `source_trust` weights on. The field carries the
  producer's own tag across; `#[serde(default)]` gives payloads written before
  it existed exactly the behaviour they had.

  **The obvious assertion cannot prove it.** A `"file"` tag and an `"agent"` tag
  both clamp to `MAX_AI_CONFIDENCE` — `clamp_confidence` separates `USER_SOURCE`
  and nothing else — so a confidence check on the watcher's own payload is green
  whether the tag survived or was overwritten. The guard is a `USER_SOURCE`
  payload at 1.0, the one tag the clamp distinguishes, and it was
  mutation-verified: restoring `AGENT_SOURCE` in the drain fails with
  `conf_beta want 1.0000, got 1.0500`. A test that cannot fail on the bug it
  names is decoration, and this one would have been.

  **The backstop's first version fed itself.** The default watched root is the
  cwd; the default intake is `.kern/intake` under it. So the durable write put a
  file in the tree that produced it — the watcher read it back, parked a payload
  wrapping that payload, and repeated. Measured against the default config from
  one seed edit: **283 payloads in 60 seconds, largest 1.77 MB, against 0 on the
  pre-change build.** `IgnoreRules` hardcoded `.git` and nothing else. Closed by
  giving it host-supplied denied prefixes and passing the resolved `intake.dir`
  and `data_dir` — named by the host, since that crate must not know what kern
  is. `effective_roots` now pins a relative root to `cwd`, because a relative
  root makes event paths relative while the denied prefixes are absolute, and
  two coordinate systems is how the check silently matches nothing. Filed as
  item 99: the loop is closed by enumerating two directories, not by an
  invariant.

  **The e2e's `STALL_MARKER` is load-bearing and that was measured, not
  asserted.** The worker persists the graph after every committed job, so
  against an instant fake LLM the "killed mid-distill" window is microseconds.
  Half (b) run alone against a pre-change build **passes** with `STALL_SECS = 0`
  (4.6s, 3.8s, twice) and **fails** with the stall (90s of retries). Without the
  stall the file would be green on the bug it was written for.

  Also confirmed rather than assumed: the fail-open path is genuinely reached
  and cannot double-ingest — `intake_direct` is tmp-then-rename and the drain
  reads only `*.json`, so no half-success both parks and re-submits; a new test
  drives it with a regular file where the intake wants a directory. And the
  `intake.enabled` gate is the right one: `drain_direct_once` needs no reason
  LLM, so `tool_ingest`'s stricter `enabled && !reason_url.is_empty()` would
  have refused to park on a reason-less host whose drain works fine.

  Decided by: verify-before-claiming — every number here is from a run against
  two built binaries, and the three claims that survived (`source_tag` is
  unprovable by confidence, the stall is load-bearing, the fail-open path is
  live) are the ones that were re-measured rather than re-read.

- 2026-07-22 — **The client authenticates the server, before it says anything.**
  A `kern.sock` client used to present the graph's `mcp-token` to whatever was
  bound at the path. With no `XDG_RUNTIME_DIR` that path is
  `/tmp/kern-<tag>-<user>.sock`, which any local user can bind first, and the
  socket token is the HTTP token — so the squat was an HTTP-side compromise.
  `connect_kern` now refuses an endpoint this euid does not own, and refuses it
  before `present_auth` writes frame 1. Item 24 residue 3 is closed on the
  disclosure axis; the daemon-side denial of service is not, and stays filed.

  **Two checks, because one is not enough.** `require_owned_by_caller` stats the
  endpoint — both the name and what it resolves to must be ours. That is the
  cheap half and it is racy on its own, and the race is not exotic: the window
  is opened by *our own daemon*, since `Drop for LocalListener` unlinks the
  socket on every shutdown and the stale-rebind path unlinks it too. A name that
  stats as ours can be free a microsecond later and rebound by somebody else
  before the `connect` lands, and an attacker only has to wait for a restart.
  So `require_peer_is_caller` reads `SO_PEERCRED` off the connected socket
  afterwards — the uid the kernel recorded when the peer called `listen`, which
  no rename can move — still ahead of the token frame. The stat stays in front
  because refusing without opening a connection yields a message naming the
  squatter's uid.

  **The `NotFound` carve-out, and why it is not a bypass.** A missing path
  returns `Io`, not `UntrustedEndpoint`, so `route()` still reads it as
  `NoDaemon` and every daemonless CLI invocation behaves as before. It cannot be
  turned into a hole because the carve-out only changes the *variant* of an
  error that is returned either way: `require_owned_by_caller(path)?` propagates
  on `NotFound` exactly as it does on a refusal, so the connect never happens
  and `present_auth` is never reached. Verified against a live binary — with no
  socket, `kern claim-kind add` still writes locally; with the path pointing at
  a root-owned file it prints `refusing endpoint`.

  **Two defects found reviewing the first cut.** The refusal message printed the
  *link's* uid on a target mismatch, so a symlink we own aimed at root's socket
  read `owned by uid 1000, not 1000` — the two cases are reported separately
  now. And the reported mutation score was wrong: neutering the stat check fails
  **6** of 6 targeted tests, not 4 of 10; the earlier count was nextest's
  fail-fast truncating the run, not four survivors.

  Decided by: name-the-tradeoff — the `NotFound` carve-out trades a precise
  error for the ordinary no-daemon path staying quiet, and it is safe only
  because the connect is skipped either way; fix-the-root — the stat-vs-connect
  TOCTOU was named and left in the first cut, and a check whose own daemon opens
  the race window is false confidence, so `SO_PEERCRED` was built rather than
  filed. What remains genuinely unclosed: the squatter still wins the
  `AlreadyRunning` probe and the real daemon still stands down, and deleting the
  peer-check *call* is a mutation no test catches, because catching it needs a
  socket served by a second uid.

- 2026-07-22 — merged item 21's `promote`, and corrected two claims that went
  stale between the branch starting and the merge landing. The branch opened at
  00:17Z with item 24 open, so it wrote "`promote` ships on an unauthenticated
  socket, deliberately" — an honest statement of a real tradeoff at the time.
  Item 24 merged at 5ab9254 while it worked. Both the entry and item 21's
  ROADMAP text now say what is true: the socket authenticates the *connection*,
  and cannot tell one same-uid caller from another, which is the part `promote`
  actually rides on.

  This is the second merge where a branch was **older than the fact rather than
  wrong**, and the two need different handling. A wrong claim gets corrected. A
  stale one gets its context preserved — the entry keeps the parenthetical
  saying it was written while item 24 was open, because the decision to ship
  `promote` without waiting was correct on the information available, and
  rewriting it to look prescient would lose why it was a tradeoff at all.

  Mechanically it is the same trap as the anchor rot: parallel cycles produce
  text that is accurate when written and false when merged, and nothing in
  `docs-check` can see it — the words are all real, they just describe a repo
  that existed an hour ago. Only reading the sentence against current master
  catches it.

  Decided by: verify-before-claiming — "was true when written" and "is true now"
  are different claims, and a merge only preserves the first.

- 2026-07-22 — item 21's review lifecycle got the caller-facing surface it was
  missing, and the item closed. `promote` (MCP tool + `kern promote <id>` routed
  through `route()`) and `exclude_pending` (query schema + `QueryArgs` + `kern
  query --exclude-pending`) shipped as one slice, because neither half is usable
  alone: the state was writable only by ingest and readable only by a filter no
  caller could set.

  **Three decisions worth recording.**

  *The unreachability was one layer deeper than the item said.* Item 21 recorded
  that `exclude_pending` had no surface. It also turned out that `[ingest]
  review_policy` — the knob that decides what is held in the first place — could
  not be set from a `kern.toml` at all: `Config::load_with_user` refused the whole
  `[heat]`/`[ingest]`/`[retrieval]` tables as preset-managed. So the hold half was
  unreachable twice over, and no e2e was possible. Fixed at the root rather than
  worked around: what a preset owns is *tuning*, and `Preset::apply` writes exactly
  one key in that table. `[ingest]` now accepts `review_policy` and nothing else.
  Decided by: fix-the-root.

  *`promote` ships on an unauthenticated socket, deliberately.* It is a wider
  claim than `intake drain` — draining asserts no authority, releasing a held
  claim is a curation-authority decision. Taken now because the alternative was
  shipping neither half, and a host that enabled `review_policy` today would
  strand every claim it held. The dependence is stated on the tool description,
  on `cmd_promote`, in `FEATURES.md` and in the user-facing `configure.mdx`:
  promote's authority rides on whatever item 24 lands.
  Decided by: name-the-tradeoff.

  *`promote` matches ids exactly, like `forget` and `degrade`.* The CLI prints
  12-char short ids, so a curator must round-trip through `kern get` — the same
  friction every other mutation has. Consistency beat convenience; making one
  verb resolve prefixes would have made the id rule mean two things.

  Coverage: `e2e/test_review_lifecycle.py` walks the whole loop twice, once
  against a blinded serving daemon and once with nothing serving. Unlike item
  18's `principals`, this needed no JSON-RPC client — the policy is ordinary
  config and both verbs are subprocess-visible.

  The config loosening was then attacked rather than trusted, because it is a
  guard relaxed mid-slice to make its own test pass. Against a real binary:
  `[ingest] dedup_threshold`, `review_policy` beside a tuning key, an unknown
  key, `[heat]` and `[retrieval]` are each still refused by name. Widening the
  allowlist to admit the whole table kills two tests
  (`preset_managed_sections_refuse_to_load` and
  `a_real_kern_toml_can_set_review_policy_and_nothing_else_in_ingest`), so the
  exception is pinned in both directions. And the guard was never what made the
  preset win: `Preset::apply` runs *after* the file deserializes, so with the
  allowlist forced wide open a file-set `dedup_threshold = 0.5` still loads as
  the preset's `0.98`. The guard is a refusal to let a `kern.toml` read as
  though it tunes; it is not the mechanism. `IngestConfig` has exactly two
  fields, so the allowlist admits precisely the one the preset does not own.
  934 tests before, 941 after; recall@1 0.9306 / recall@5 0.9722 / MRR 0.9471,
  unmoved.

- 2026-07-22 — swept every `**Gaps.**` block in `FEATURES.md` against
  `ROADMAP.md` looking for more defects of item 98's kind — real, stated in a
  description, carried by no item. **Found none.** Recording the negative result
  with its method, so the next pass does not re-raise the same alarm.

  The alarming-looking number is a false signal: **16 of 24 gap blocks cite no
  ROADMAP item.** Spot-checking the two most concrete showed both are carried
  anyway — "an entity dropped past the cap is gone" is closed item 5 (intended,
  and counted by `unspilled_drops` on three health surfaces), and
  `cmd_hub_merge`'s unguarded write is carried in item 9's section. Citation
  style is not coverage, and counting citations measures the wrong thing.

  So item 98 was a genuine one-off rather than the tip of a pattern, and it was
  found by reading a neighbouring file rather than by any rule. Two heuristics
  were tried and both came up empty: word-overlap between gap sentences and the
  roadmap, and absence of an explicit item reference. The first is too loose to
  discriminate, the second measures formatting.

  Worth keeping the shape of the question even though the answer was "nothing
  here": **a defect stated only in a present-tense description is not scheduled
  work**, and that failure mode has produced three real items this run (95 out of
  item 20's prose, 97 out of item 28's, 98 out of FEATURES §13). The sweep says
  the backlog is currently clean of it, not that the failure mode is gone.

  Decided by: verify-before-claiming — "16 of 24 uncited" looked like a finding
  until two of them were opened, and neither was.

- 2026-07-22 — item 24's title said "RPC socket has no auth" while its body said
  "mostly closed 2026-07-22". Retitled to what is actually true: the connection
  authenticates, the caller does not — same-uid callers remain
  indistinguishable, which is the half items 9 and 18 were waiting on. Fourth
  time a heading has outlived the defect it names, and slice selection reads
  headings.

  Also filed item 98, which `FEATURES.md` §13 stated and no item carried: **the
  pre-auth frame is unbounded and untimed.** The one thing reachable before the
  token is checked is the one thing with no cap on it. A frame declaring a huge
  length makes the daemon allocate for a peer that has proven nothing; a
  connection that opens and sends nothing holds its slot forever, and item 24
  deliberately places the auth check before the handler exists, so that stall
  costs a session that will never be authorised.

  Not escalated, because `harden_socket` sets the socket `0600` and the peer is
  therefore already a same-uid process. But that is exactly the attacker item
  24's own residue says it cannot police — so "only a same-uid caller can do it"
  is not mitigation here, it is a restatement of the open gap.

  The pattern worth naming: **`FEATURES.md` described this correctly the whole
  time.** The gap was not knowledge, it was that a limitation living only in a
  present-tense description is not scheduled work. `ROADMAP.md` is the only file
  that says what is left, so a real defect stated anywhere else is a defect
  nobody will action. That is the third time this run — item 95 out of item 20's
  prose, item 97 out of item 28's — and all three were found by reading the
  neighbouring file rather than by anything automated.

  Decided by: fix-the-root — the recurring failure is stating a defect somewhere
  that is not the list of what is left.

- 2026-07-22 — **`kern.sock` authenticates.** One `AuthReq` frame carrying the
  graph's `mcp-token` is compared in constant time before any `KernRpc` method
  dispatches, and the Windows named pipe is created with an owner-only SDDL
  instead of the default descriptor. Item 24 is narrowed to a residue, not
  closed.

  **The defect the e2e corpus caught, and nothing else would have.** The socket
  is keyed by the **root** (`Endpoint::kern_for` hashes the path) while the
  token was keyed by the **data_dir**. Those are the same directory right up
  until `kern.toml` is repointed under a live daemon — which is exactly the
  blinding technique `e2e/` uses — and then the daemon keeps serving out of the
  store it opened at boot while a later CLI, reading the new config, hunts for
  the secret in a directory that daemon never wrote to. Every unit test passed:
  they all built root and data_dir together, so the two keyings were
  indistinguishable. `rpc::token_for` now searches the configured store first
  and the root's conventional `.kern/data` second. The fallback is safe in the
  only direction that matters: it changes what a *client presents*, never what
  the *server accepts*, so a bad guess produces a refusal and can never produce
  an admission.

  **Why `principal` is recorded and not enforced.** The frame declares `cli`,
  `mcp` or `hub`, and nothing consults it. A shared secret proves a **uid**; the
  CLI, the `kern mcp` proxy an agent drives, and the hub all run as the same uid
  and can all read the same file. There is no fact on that connection that can
  separate them, so enforcing a self-asserted principal would be a permission
  check a caller writes for itself. Recording it puts the field where items 9
  and 18 need it without pretending it carries weight it does not.

  **The gate was mutation-tested, and the first round of tests was decoration.**
  Making verification always succeed did not fail the no-token test: an open
  gate still *eats* the first frame, so one tool call cannot tell "consumed as a
  handshake" from "consumed and refused". Offering the call twice distinguishes
  them — past an open gate the second one lands — and re-running the mutation
  confirms the two-attempt shape is what kills it. A second mutation found a
  second decoration: every wrong token in the suite differed in *length* from
  the right one, so `ct_eq`'s length short-circuit refused them all and gutting
  the byte compare killed nothing at the gate. Both suites now offer a
  same-length token differing in the final byte, and a negative case runs over a
  real Unix socket rather than an in-process pipe.

  Windows is typechecked (`cargo check --target x86_64-pc-windows-msvc -p
  trnsprt`, with a deliberate type error proving the check is not vacuous) and
  has never executed. Said plainly in `ROADMAP.md` item 24 rather than implied.

  Decided by: fix-the-root — the root-vs-data_dir keying was the defect, and
  repointing the test would have hidden it; name-the-tradeoff — reusing the HTTP
  `mcp-token` means a socket-side disclosure is an HTTP-side compromise, which
  is why item 24 stays open.

- 2026-07-22 — merged item 21's review lifecycle. 203 + 1 + this one = 205. The
  merge broke **thirteen** citations, the worst single hit yet, and every one was
  a second-order effect: adding `ReviewState` to `Entity` shifted `types.rs`,
  which shifted every anchor below it, in four documents that never mentioned
  review at all.

  Fixed by locating each cited symbol and repointing — `Entity` 280→293, `Reason`
  428→442, `Kern` 471→485, `ReasonKind` 77-86→90-99, `matches_filter` 216→235,
  `job()` 38→40, and so on. Mechanical, but thirteen of them, and `docs-check`
  cannot do it: it nominates a mismatch, it cannot find the new line.

  This is the clearest argument yet for item 93's unbuilt half. The nominator
  works — it caught all thirteen and stayed silent about the ~750 anchors that
  were fine — but it converts a silent corruption into a manual chore that scales
  with how much a struct moves. **A symbolic anchor
  (`` `src/base/types.rs#struct Entity` ``) would have needed zero of these
  edits.** The tax is now measurable: one field added to one hot struct cost
  thirteen repoints across four files.

  Worth noting what this does NOT argue for: leaving the anchors as line numbers
  and lowering the bar. Every one of the thirteen was genuinely wrong and would
  have sent a reader to unrelated code — `Entity` cited at `self.section()`,
  `Reason` at `observe_support`. The chore is real because the breakage is real.

  Decided by: name-the-tradeoff — the nominator buys correctness at the price of
  manual repointing, and the price is now large enough to fund the fix.

- 2026-07-22 — item 21's review lifecycle lands three parts of four, and the
  missing fourth is the one that makes the feature safe to turn on.

  Shipped: `ReviewState` on `Entity` behind a `FORMAT_V7` bump with old stores
  rejected rather than defaulted; `exclude_pending` as a `QueryOptions`
  predicate; and source-scheme review policy in `IngestConfig`, with unknown
  schemes rejected at load.

  **The default is `Active` and that was the whole risk.** Pending-by-default
  would have made every existing ingest path silently non-retrievable — a
  behaviour change wearing a schema addition's clothes, which craters recall
  instead of failing loudly. Active-by-default means the feature is inert until a
  host opts in. Recall is unmoved at 0.9306 / 0.9722 / 0.9471, which is the
  evidence that inertness is real rather than intended.

  **Not shipped: `promote`.** No such arm exists in the MCP dispatch. A host can
  therefore configure a scheme to arrive held, and can filter held rows out of
  retrieval, but has no supported way to release one. Shipping the hold without
  the release is worse than shipping neither, so item 21 is retitled to say so
  and carries a do-not-enable warning rather than a completion note.

  The docs are written by me rather than the slice: its agent stalled twice, the
  second time after fixing the compile error that blocked verification but before
  touching `ROADMAP.md` or this file. Code and tests were verified green
  independently — 927 tests, `exclude_pending` pinned in both directions, the
  format rejection pinned by the same test that pinned `FORMAT_V6`. What was
  missing was only the record, and a slice that stalls before recording leaves
  work that looks finished to `git status` and unfinished to everyone else.

  Decided by: name-the-tradeoff — an inert default is the safe half of a feature
  that can hide data, and the half that can un-hide it is not there yet.

- 2026-07-22 — merged item 94's lexical indexing of dedup alternate wordings.
  201 + 1 + this one = 203.

  The ROADMAP conflict is worth a sentence because it is the first one where
  both sides were *correct when written*. `cycle/1` branched before item 28's
  sparse adjacency landed and carried the accurate-at-the-time text "a
  propagation still takes 79.7s at N=4096"; master carried the closure that
  replaced it. Nothing was wrong on either side — the branch was simply older
  than the fact. Took master's.

  That is the ordinary cost of three parallel cycles against one file, and it is
  cheap: git flagged it, both versions were legible, and picking took one read.
  The expensive failures this run were the ones git could NOT flag — a stale
  `index.lock` that produced an empty staged merge with a clean `git status`, and
  a hand-spliced conflict that dropped four entries. Textual conflicts are the
  visible tax; the invisible ones needed a counting rule to catch.

  Decided by: verify-before-claiming — "both sides look right" is resolvable by
  asking which is newer than the code.

- 2026-07-22 — item 94 closed: a deduped near-duplicate's alternate wording now
  reaches the lexical index, as part of the survivor's one document.

  The item was real — the first of this run's slices where the premise survived
  contact. Measured before touching anything: the wording sits on a `Rephrase`
  reason with `vector.len() == 0`, `reason_idx` empty, and the lexical index
  answering nothing for a term only the merged document used. The query that
  proves it: over a corpus where twenty fillers sit nearer the query vector,
  `velocipede outbuilding` returned twenty fillers and never the survivor that
  had swallowed those exact words.

  **The remedy the item named would have made it worse.** It asked for "one
  `lex.insert` of the rephrase text against the survivor's id". The index is
  keyed by entity id and replaces on insert, so that would have swapped the
  survivor's own wording out for the alternate's — a lateral move, not an
  addition. Shipped instead: one document per entity, statements plus every
  `Rephrase` text, which also settles "does it appear twice" structurally rather
  than by a dedup rule.

  Lexical only, no vector for the alternate: both dedup gates reach
  `merge_duplicate` without an embedder, and `Mode::Hybrid` never reads
  `reason_idx` anyway. The dense gap was the small one — two texts merge only
  when their vectors are already within threshold.

  **What did not move, and the honest reason.** Recall is unchanged at
  0.9306 / 0.9722 / 0.9471: the e2e corpus has no near-duplicate pair, so no
  `Rephrase` is minted and every document is byte-identical. Two probes were
  written and thrown away for testing nothing — `kern search` turns out to be
  pure vector and never reads the lexical index, and an "appears once" assertion
  passed while reverted because the shared words carried it. Both were caught by
  the revert step, not by reading. The residual is named in the item: the fix
  makes the wording a *candidate*, and `fuse_hybrid_seeds` then re-ranks every
  seed by the query cosine that failed to find it, which is item 61's question.

  Decided by: verify-before-claiming — the item's own proposed fix was a claim
  about code, and reading the index it named is what showed it backwards.

- 2026-07-22 — merged item 28's sparse adjacency, but only after catching a
  merge that would have thrown it away. 198 + 2 + this one = 201.

  `cycle.sh reap` failed and left `.git/MERGE_HEAD` behind with a **staged,
  conflict-free, entirely empty** merge: `git status` clean, `git diff --cached
  HEAD` zero lines, `src/gnn/sparse.rs` absent from the index while present in
  the branch. Committing that would have recorded item 28 as merged while
  discarding every line of it — a 6.3x speedup and a new module, gone, with the
  history saying they landed.

  The cause was a stale `.git/index.lock`: another process held it mid-merge, so
  git could not write the index and produced an empty result rather than an
  error the caller surfaced. `git merge --abort` then failed too ("remove the
  file manually to continue"). Clearing the lock and re-running gave the real
  merge, with the two genuine conflicts it should have had.

  What caught it was arithmetic, not reading. The staged tree carried 198
  recorded-decision entries — exactly HEAD's count — while the branch had two of
  its own. A merge that adds a branch's work cannot leave the entry count
  unchanged.
  That check exists because a hand-spliced conflict silently dropped four
  entries earlier in this run; it has now caught a second, larger failure of a
  completely different kind.

  Worth stating plainly: **a clean `git status` after a merge is not evidence the
  merge did anything.** The reap tooling reported failure and left a state that
  looked finished, and every visual check agreed with it.

  Decided by: verify-before-claiming — the count disagreed with the appearance,
  and the count was right.

- 2026-07-22 — filed item 97: **the e2e harness cannot exercise the GNN at all.**
  `DEFAULT_MIN_THOUGHTS` is 128 and the recall corpus is 36 facts, so no
  propagation runs in `e2e/` in either column of any A/B. Every "recall
  unchanged" reported for a GNN change to date describes code that never ran.

  Found by the item 28 slice, which had been handed a recall gate as its safety
  bar and reported that the gate did not exist rather than banking the green.
  That change shipped on a bit-identity proof instead — asserted over `to_bits()`
  across two orientations, four widths and three graph shapes — which is what
  actually carries it.

  Filed rather than left in that item's prose, because the slice kept its edit
  inside item 28's section as instructed and a gap living inside a neighbouring
  item is one nobody schedules. Same reason item 95 was filed out of item 20 last
  night.

  The uncomfortable part is that the gate was mine. The brief said "recall must
  be EXACTLY 0.9306 / 0.9722 / 0.9471" and treated that as the bar the change had
  to clear; it was satisfied by a suite that never invoked the code. A bar nothing
  can fail is not a bar, and this one had been quietly passing for every GNN
  change in the file.

  Decided by: verify-before-claiming — a green gate is a claim about coverage,
  and this one was never checked.

- 2026-07-22 — item 28's remainder closed: the GCN aggregation reads a sparse
  adjacency instead of a dense N x N one. 73.4 s → 11.6 s at N=4096, 20.5 s →
  7.4 s at 2048, 5.4 s → 3.9 s at 1024, and bit-identical output.

  Both arms were run back to back in one session rather than compared against
  the number already in the item, because the two no longer share a bottleneck
  and a cross-session comparison would have flattered the result. Dense is
  bandwidth-bound on a 134 MB matrix and shrugs off CPU contention; sparse is
  CPU-bound on the linear layers and does not. On a busy machine that is 6.3x at
  N=4096; on an idle one the same after-numbers are 1.6 / 3.3 / 6.6 s, or 12.1x.
  6.3x is the floor and the number quoted above.

  Measured before implementing, and the measurement moved the target. The item
  blamed `normalized_adjacency` *materialising* the dense matrix. Materialising
  was the smallest of the three dense costs at N=4096 — 11.1% of the
  propagation, against 65.5% for the multiply and 12.8% for a per-backward
  `transpose` the item never named. All three are consequences of the same dense
  representation, so the remedy the item proposed was right; its explanation was
  not, and optimising only the named term would have bought 11%. That is the
  fourth slice running where an unmeasured cost estimate inside an item pointed
  at the wrong term.

  **The decision this item was held open for was whether a faster propagation is
  worth moving ranking. It is not a trade, because it is bit-identical.** The
  aggregation is the only computation that changed, and two properties make the
  sparse product agree with the dense one exactly rather than closely: the entry
  is the same `1.0 / (sqrt(di) * sqrt(dj))` expression over a degree the dense
  form reaches by summing that many exact `1.0`s, and columns ascend inside a
  row, so the sparse product visits the same nonzeros in the same order. What it
  skips are exactly the stored zeros, and `x + 0.0 * b == x` for a
  `+0.0`-seeded accumulator and finite `b` — item 26's argument in the same
  shape. Identical inputs through identical downstream code give identical
  outputs, so `gnn_vector` and therefore recall cannot move.

  The equivalence is asserted over `to_bits()`, deliberately, not a tolerance. A
  1-ULP tolerance would have accepted writing the normaliser as
  `1.0 / (di * dj).sqrt()`, which is mathematically the same expression and one
  bit different — that mutation was run, and the bit assertion caught it where
  any relative tolerance above 1e-15 would not have.

  **The recall gate the item asked for does not exist**, and that is worth
  recording rather than quietly passing. `pytest -q -s e2e` is green and recall
  is 0.9306 / 0.9722 / 0.9471 before and after, identical — but its corpus is 36
  facts against a `min_thoughts` floor of 128, so no propagation runs in it in
  either column. It is a true green about nothing. The proof above is what
  carries this change; the e2e run only shows nothing else broke.

  Two things left unbuilt on purpose. The dense `normalized_adjacency` stays in
  `src/gnn/graph.rs` though nothing in production calls it: it is the reference
  the equivalence test compares against, and a reference that lives in the test
  can drift from the thing that shipped. And the trainer's single `std::thread`
  was chosen because "each training allocates a dense `num_entities^2`
  adjacency — 134MB at N=4096"; that is now 0.33 MB, so the reason is gone and
  the concurrency choice is re-openable — but on its own evidence, not as a
  rider on this one, which is the mistake this item was created to avoid.

  The `FEATURES.md` edit is line-count neutral on purpose: eleven `ROADMAP.md`
  anchors cite it by line number below the edited block, and item 93 is open
  precisely because nothing catches that shift. Inserting five lines nominated
  eight of them.

  Decided by: verify-before-claiming — the item's named cost was 11% of the
  cost, and the gate it demanded was measuring nothing.

- 2026-07-22 — merged item 92's closure. 195 + 2 + this one = 198, by union
  rebuild; the duplicate-94 renumber survived the merge and `### 96` is now
  unique.

  Two slices in a row corrected this file rather than the code, and both found
  the same shape of error: **a number written down once and then cited as
  established.** Item 26's slice found the instrument it inherited was unfair —
  it charged the full-width reference a sort the confined path never pays, which
  is where the recorded 1.43x came from; measured fairly it is 1.19x. Item 92's
  slice found the recorded clock constant was one sample — the invariant is a
  ~3.8% rate, not a 2.8 s step, and the step scales with however long the sync is
  delayed.

  Neither number was invented. Both were real observations that stopped being
  observations the moment they were written without their method. The habit that
  catches it is already in every brief — measure before implementing — but these
  two show the second half: **measure again when the number came from a previous
  pass**, because a figure in this file carries no error bars, no method, and no
  date unless someone put them there.

  Both slices also corrected records I wrote. That is the system working in the
  direction it is supposed to: the loop has now caught roadmap items, subagent
  claims, merge subjects, and the orchestrator's own notes, using the same check
  each time — read the thing itself.

  Decided by: verify-before-claiming — a recorded measurement is a claim like any
  other, and it ages.

- 2026-07-22 — two of my own records were wrong and the item 92 slice found both.

  **The clock figure was a sample reported as a constant.** I recorded "steps
  `CLOCK_REALTIME` backwards ~2.8 s every ~30 s" from one incidental observation.
  Measured properly by two independent methods, the invariant is different:
  **realtime runs ~3.8% slow and the sync repays the whole accrued drift in one
  backward jump.** A 300 s window gave 9 steps averaging -1.243 s at 32.25 s
  apart; a later window stretched to 47.45 s and its step grew to -1.816 s.
  1.243/32.25 = 3.85%, 1.816/47.45 = 3.83%. The rate is constant; the step is
  not. My 2.8 s was real — it just needed a sync delayed to ~73 s, which is
  exactly what a long preceding test buys, and which is why the flake correlated
  with `test_recall` rather than with load.

  **The fix I asked for had already shipped.** I wrote that
  `e2e/test_retention.py` "wants the same treatment". It got it in `588e53a`,
  seven hours before I wrote that sentence. `wait_past_deadline` has waited on an
  absolute realtime target with a monotonic cap since then. I wrote that update
  from the item rather than from the file — the precise failure this loop has
  caught in six roadmap items and now in me.

  The slice therefore shipped **no code fix, and the absence is the result.** It
  also disproved the alternative I proposed: an injected instant loses because
  `drop_expired` returns early whenever `valid_at`/`as_of` is set, so the test
  would exercise the filtered reader while production expiry rides the
  unconditional pass.

  Its method is the part worth keeping. Waiting for this flake is not a test —
  the *reverted, defective* code passed 12 consecutive runs, which is exactly why
  three observers could not reproduce it. So it constructed the step with an
  `LD_PRELOAD` shim subtracting a monotonic-derived offset from `CLOCK_REALTIME`
  only, consistent across pytest and every `kern` subprocess. Reverted: 0 of 5.
  Shipped: 5 of 5. That the Rust binary also expired correctly proves the shim
  reached past Python.

  Decided by: verify-before-claiming — I asserted a constant from one sample and
  a defect from a document, and the check that would have caught either was
  reading the thing itself.

- 2026-07-22 — item 92 closed with no test change: `e2e/test_retention.py` was
  already carrying the fix, and the mechanism's constant was wrong.

  The item asked for a fix and named the file. The file already waited on the
  wall clock and polled for the drop, and had since 2026-07-21 17:57 — seven
  hours before the commit that updated the item to say it did not. Nothing was
  implemented; the record was.

  Measuring the clock rather than trusting the entry moved the number too. Two
  methods sharing no code path — a 50 ms two-clock sampler, and `/proc/uptime`
  against `date(2)` — agree on **-1.243 s every 32.25 s**, not the recorded
  2.8 s every 30 s. And the two figures disagree about *which quantity is
  constant*: a period that stretched to 47.45 s carried a step that grew to
  1.816 s, the same 3.85%. Realtime runs ~3.8% slow and the sync repays the
  accrued drift in one jump. A margin is unsafe because the loss scales with the
  wait, not because a fixed 2.8 s might land in it.

  That is what made the flake unreproducible, and the pre-fix shape proves it:
  12 of 12 green interleaved with `test_recall.py` against the host's own clock.
  A revert-check that waits for this flake cannot fail on demand, so the step was
  built instead — an `LD_PRELOAD` shim warping `CLOCK_REALTIME` as a pure
  function of `CLOCK_MONOTONIC`, so driver and subprocesses share one warped
  clock. At 2.8 s every 5 s the pre-fix shape fails 5 of 5 on its original
  message; the shipped shape passes 5 of 5, and the whole file passes.

  An injected `valid_at` was declined: the CLI has no such flag, and
  `drop_expired` short-circuits whenever one is set, so it would have tested the
  filtered reader instead of the unconditional pass production expiry rides.

  Decided by: verify-before-claiming — the item claimed a file was unfixed and
  a constant was 2.8 s, and both claims were cheaper to check than to inherit.

- 2026-07-22 — item 26's full-reach regression closed: the power iteration runs
  full-width loops once the reached set is closed and covers 90% or more of the
  graph, which is past where the confined walk stops paying for itself.

  Reproduced before it was fixed, and the item's own number did not survive that.
  Its instrument charged the full-width reference a 100k-row sort the confined
  path never pays, so "1.4× slower at full reach" was two costs added together;
  the same instrument on this box read 1.19× today. A fair A/B — one function
  against itself, same top-k tail, only the loop body differing, over graphs whose
  edges stay inside a block of known size so reach moves while edge count does not
  — puts the real penalty at 1.06–1.29× from 88% reach upward and confirms the
  regression is genuine. The crossover is ~80% reach and holds at out-degree 4, 8
  and 16 alike; at out-degree 1 the reached set does not close inside 25 iterations,
  so the switch cannot fire and does not need to.

  The tradeoff taken: a second copy of the loop body, which must stay bit-identical
  to the first forever, bought for that 1.06–1.29× wherever the seeds saturate. It
  is paid for rather than hoped for — the existing bit-identity test now runs its
  whole matrix through both bodies and across two graphs placed either side of the
  threshold, and fails if only one body was ever walked. The threshold is 90 and
  not the measured 80 because the two errors are not symmetric: switching late
  gives back a 1.3× band, switching early costs up to 1.22× on graphs the confined
  walk still wins.

  What is left of item 26, and its whole title now, is the four N-sized buffers
  allocated per call. Removing them means a sparse rank vector, and that has to be
  argued against bit-identity first — a hash map does not iterate in the ascending
  index order the `+0.0` argument depends on.

  Decided by: name-the-tradeoff.

- 2026-07-22 — merged item 27's batched GC eviction. 192 + 1 + this one = 194,
  by union rebuild.

  Worth a line on what the last three slices have in common, because it is not
  what the roadmap predicted. Item 31 shipped **zero source changes** — routing
  fan-out is a slope, ~2% of ingest, and the cosine comparison the item blamed
  measured at zero. Item 27 shipped a 213x speedup on a cost the item had ranked
  *fourth* in its own list, after three earlier bullets were each withdrawn by
  measurement. Item 29 refused its remedy outright.

  So the ranking inside a multi-bullet item has been wrong about as often as the
  ranking between items. Item 27's four bullets closed in the order 3, 4, 1, 2 by
  value delivered, and the two that mattered were the two nobody had measured.
  The pattern is not "roadmap items are unreliable" — it is that **an unmeasured
  cost estimate is a guess wearing a number**, and the file contains many, at
  every level of nesting.

  What keeps this honest is cheap and already habitual: every slice measures
  before it implements, and reports the measurement even when it kills the slice.
  Three of the last four did exactly that.

  Decided by: verify-before-claiming — the ordering within an item deserves the
  same scepticism as the ordering between items.

- 2026-07-22 — item 27 closed: a GC sweep pays one LMDB commit, not one per
  victim. 616 s → 2.9 s at 80 000 victims; 4 367 ms → 35 ms at the 100k/800 point
  the item was written around.

  Verified before implementing, because three of item 27's four bullets had
  already ended somewhere other than their title pointed. `cold_spill` and
  `cold_put_all` encode the same rows and issue the same two `put`s per row and
  differ only in where the transaction boundary sits, so an A/B over identical
  batches attributes the cost to the commit and nothing adjacent: 9.18 ms per
  row against 0.21 ms at 100 victims, 6.80 ms against 0.05 ms at 20 000, with
  the cold tier under its cap in both columns so no trim pass fired in either.

  The item said the open question was not how to batch but what a batched
  failure means, and that is the decision recorded here. **All-or-nothing was
  rejected.** Cold GC is the only bound on hot-graph size, so a permanently
  un-encodable row that took the whole sweep down with it would wedge that bound
  every hour, forever — and it would buy nothing in exchange, because a batch
  that fails has written nothing and loses no data under either policy. So a
  failed batch falls back to the per-victim loop it replaced and the retention
  semantics are exactly what they were: the bad row stays hot and is retried
  next sweep, every other victim is still collected. The same fallback absorbs a
  batch too large for one LMDB transaction by finishing the sweep slowly instead
  of not at all.

  Two costs accepted and named in the item: the batch clones every victim entity
  before committing (~80 MB transient at 80 000 victims, of data already resident
  and about to be freed), and one write transaction is now held for a whole sweep
  rather than V short ones — a single ~2.9 s hold against 616 s of intermittent
  ones, so contending writers wait strictly less in total but a single flush can
  block longer once.

  The equivalence is the whole claim, so it is a test: the batched path evicts
  exactly the victims the per-victim path evicted and spills exactly the same
  rows, over a 200-entity mixed population with 80 victims — a one-victim sweep
  is a single commit either way and would have proved nothing. Fact immunity is
  pinned separately against the batch, which is a second place it has to hold
  now that a victim list is handed to the store wholesale.

  Decided by: name-the-tradeoff — batching was never in doubt; which failure
  mode to buy with it was the only real question.

- 2026-07-22 — `FEATURES.md` stated the acceptance radii as
  `KERN_INNER_RADIUS=0.15, KERN_OUTER_RADIUS=0.35`; `src/base/constants.rs:40-41`
  say 0.35 and 0.75. Both numbers wrong, by more than a factor of two, on the
  constants that decide whether an entity joins a kern or spawns a new one.

  Found by the item 31 slice while measuring routing, and flagged rather than
  fixed because it sat outside that slice's section. Fixed here in the same
  worktree, with the source anchor added so the next drift is nominated instead
  of merely wrong: `docs-check` verifies a cited line still says what the citing
  sentence claims, and this line cited nothing at all.

  Worth the entry because of what it says about the check's reach. Nine hundred
  tests, an anchor nominator and a citation checker all passed over this for the
  entire life of the file — none of them compares a documented *value* against
  the constant it names. Anchored prose gets checked; unanchored prose is
  unfalsifiable. The cheap discipline that follows: when a doc states a number
  from source, cite the source line, or the number is a rumour.

  Decided by: verify-before-claiming — a stated constant is a claim, and this one
  had never been checked against the thing it claims.

- 2026-07-22 — item 31's last surviving bullet, "per-parent fan-out in routing is
  a real cliff", is retired unfixed. `tests/route_fanout.rs` (release,
  `--ignored`) prices it two ways, and the item is right about the structure and
  wrong about the cost.

  **The width is real and nothing bounds it.** Root fan-out tracks the number of
  distinct cohesive topics almost exactly: 8 topics -> 8 named children, 64 ->
  55, 256 -> 191, driving the real accept → cluster → name → promote loop.
  `GRAVITON_DEDUP_THRESHOLD` collapses only topics whose graviton names embed
  within 0.85 of each other — a fact about the corpus, not a cap. Disabling
  promotion does not shrink the width, it moves it one level down into `generic`.

  **The cost of that width is linear and small.** A child costs 0.14-0.18us
  across runs, on an accept that costs 1.4-2.1ms at 20k entities: ~2% at 191
  children, ~5% at 512, ~24% at 4096 — and a graph with 4096 distinct themes
  holds far more than 20k entities, so the real fraction is lower still. Ingest is two HNSW searches and
  two inserts; routing is a rounding error beside them. A slope, not a cliff.

  **And the scan the item blames is not where even that goes.** Running the same
  descent with the children unnamed — identical walk, cosine skipped — moves the
  per-child figure by -0.009, -0.001 and +0.003us on three runs — zero every
  time. The width is paid in two `Vec<String>` clones per descent and a linear
  resident-map probe for the generic child, not in `cosine_distance` against `graviton_vec`. That is the
  third time this item has named the wrong line; its two 2026-07-21 retirements
  were misattributions too. The pattern is that every version of it was written
  from the shape of the code rather than from a run.

  So no index over children shipped. Naming the tradeoff that was declined: an
  index is write-path work on every spawn and every rename, and what it buys
  back is a comparison that measures as free. If the slope ever matters, the
  lever is deleting the clone, and item 31 now says so instead of pointing at
  the index. Supersedes the "real cliff" claim recorded under item 31 on
  2026-07-21.

  Recorded because the first draft of this entry claimed the opposite — that
  fan-out was sublinear and `TICK_MAX_CLUSTER_SAMPLE` suppressed it. That came
  from a generator whose 32-word vocabulary made distinct topics produce
  identical graviton names, which the 0.85 gate then collapsed. It was caught
  only by trying to make the assertion fail and finding it could not be, which
  is the entire reason that step exists.

  Decided by: verify-before-claiming — the cliff was measured before it was
  climbed, the instrument that priced the width also proved the named cause was
  not it, and the first conclusion was withdrawn when its revert-check would not
  fail.

- 2026-07-22 — item 92 updated in place rather than rewritten, and retitled to
  name the mechanism: "Tests that race a backward-stepping `CLOCK_REALTIME`".
  The item had guessed wall-clock *lag under load*; the cause is the clock
  stepping backwards ~2.8 s every ~30 s, found in an unrelated file.

  The guess and the finding are both kept, because the difference between them
  is the useful part. Lag would have been fixed by widening the margin — the
  item said so. A backward step cannot be: widening only lengthens the odds,
  since the trigger recurs every half-minute regardless of how long the margin
  is. Same symptom, opposite remedy.

  It also explains the evidence that made the item look unresolvable. Six
  consecutive clean runs disproved nothing against a trigger firing twice a
  minute, and the correlation was never with load — it was with a long preceding
  test, which simply spans more backward steps. Filing that disagreement instead
  of resolving it early is why the entry needed an update rather than a
  correction.

  Decided by: verify-before-claiming — an unconfirmed mechanism was labelled
  unconfirmed, and survived contact with the real one.

- 2026-07-22 — a flake fix arrived in the MAIN checkout, not a worktree, and
  `cycle.sh reap` refused to run because of it — correctly, and that refusal is
  the reason this is a note rather than a silent clobber. Three cycles were live
  at the time; reaping into a dirty tree would have mixed an unattributed change
  into someone else's merge.

  The change itself is good and is kept.
  `the_poll_loop_resolves_its_deadline_per_pass_not_once_at_startup` slept two
  seconds on the monotonic clock and compared against `valid_until`, an absolute
  `SystemTime`. It now waits on the wall clock and restarts its marker if the
  clock steps backwards, with a thirty-second monotonic cap so a stopped clock
  fails loudly instead of hanging.

  The environmental finding is worth more than the fix: **this box steps
  `CLOCK_REALTIME` backwards roughly 2.8 s every 30 s.** That is almost certainly
  the mechanism behind ROADMAP item 92 — `e2e/test_retention.py` failing
  intermittently under load, which three observers could not reproduce on demand
  and which was filed with deliberately conflicting evidence. Item 92 guessed
  "wall-clock lag under load on WSL2". It was not lag; it is the clock going
  backwards. Any test comparing a monotonic sleep against a `SystemTime` deadline
  on this host is a coin flip, and there are others.

  What is not fine is the delivery. A writer outside the worktrees is invisible
  to the claim ledger, so two of today's collisions and now this all share one
  cause: the isolation only binds writers that go through it. The reap gate
  caught this one because a dirty tree is loud. A change committed straight to
  master would not have been.

  Decided by: verify-before-claiming — the change was verified green and kept on
  its merits, and the way it arrived is recorded separately from whether it was
  right.

- 2026-07-22 — three ROADMAP headings still named defects that had been closed,
  and the heading is the index. Items 28, 29 and 95 all carried struck-through
  bodies marked closed while their titles read "GNN training runs synchronously
  on the tick", "a spilled kern still carries two resident indexes" and "the file
  watcher bypasses clamp_confidence entirely". Every one of those is now false.

  This matters more than a tidy-up because of how the list is actually used.
  Slice selection each cycle starts with `grep "^### "` — the titles are what a
  reader scans and what a dispatcher picks from. A body that says "closed" behind
  a title that says "broken" is invisible to the only step that reads the file at
  scale, and the cost is a cycle dispatched at work already done. That nearly
  happened here: 28, 29 and 95 were all live candidates on this fire's shortlist.

  Retitled to name what REMAINS, following item 27's precedent ("GC eviction pays
  one LMDB commit per victim" after two of its four bullets closed). 28 becomes
  the 79.7s propagation cost, 29 becomes the measured refusal, 95 states it is
  closed. The rule the file needs and now has three examples of: **when an item
  narrows, the title narrows with it** — a title describing the original defect
  outlives the defect.

  Decided by: fix-the-root — the stale titles were not the error, the error was
  updating bodies without updating the index that points at them.

- 2026-07-22 — item 95 closed: ingest confidence is clamped by a guard every
  producer must pass, not by a convention each one remembers. Verified first —
  the watcher's raw `1.0` really did land on Beta(2,1) = 0.6667, a human CLI
  claim's posterior and above the 0.6500 a deliberate MCP agent gets.

  The item proposed clamping inside `Worker::submit`. That shape was too narrow:
  `intake.rs`'s `drain_document` minted a raw `1.0` through `run`, so fixing
  `submit` would have closed one of two live holes and left the identical defect
  one method over. The clamp went into `Worker`'s private `job()` instead — now
  the only place a `Job` is built, with `run_with_acl` no longer assembling one
  by hand — and every entrance takes a `source_tag` it cannot omit. A producer
  is now asked "who is asserting this?" by the compiler.

  The tag is the channel, `source.scheme()`: not `USER_SOURCE`, because no human
  asserted a file that changed on disk; not `AGENT_SOURCE`, because an agent's
  ceiling belongs to a deliberate assertion and a file appearing is not one. No
  new `"watcher"` constant, because `clamp_confidence` separates only
  `USER_SOURCE` from everything else — a second name for the same 0.95 would be
  the label-that-weights-nothing item 20 already refused. The two paths that know
  their principal name it: CLI `USER_SOURCE`, MCP and direct-intake replay
  `AGENT_SOURCE`.

  Ranking moved for `Document`s (0.6667 → 0.6500) and not at all for the recall
  corpus, which is CLI-ingested: 0.9306 / 0.9722 / 0.9471 before and after,
  bit-identical to the worst-probe list. The recorded baseline stands unchanged.
  Supersedes item 20's claim that a user-authored claim does not outrank an
  auto-ingested one — it now does, by 2.6%, which is a rounding error and not
  yet a trust model.

  Decided by: fix-the-root

- 2026-07-21 — item 83's double-storage half: one vector, shared, 185.6 MB off
  the resident total, paid for with 9% on the index walk.

  Item 29 measured that the kern map was the largest resident holder and said
  why: every vector was stored twice, once in `Kern::entities`/`reasons` and
  once in the index pointing at it. The premise was checked before anything was
  built, because six of nine slices that day ended somewhere other than their
  title pointed. It held — `index_kern_into` passed `t.vector.clone()` to each
  index and `HnswNode` kept that clone verbatim under the shipped default
  `QuantizationMode::None`, so the two really were the same floats rather than a
  normalised copy and a raw one. Under the opt-in `int8` mode the node's float
  vector was already empty; this change buys nothing there, which is worth
  knowing before someone re-measures under `kern compress`.

  `Entity::vector`, `Entity::gnn_vector` and `Reason::vector` are now
  `Embedding = Arc<[f32]>`. `Arc` beat the alternatives on the risk it does
  *not* carry: it has no `DerefMut`, so an in-place write through one holder
  that the other would see is a compile error, and the compiler enumerated the
  ~20 write sites rather than a human doing it. A slab index would have made the
  same aliasing bug silently expressible and would additionally be meaningless
  outside the owning graph, where entities are cloned, merged from peers and
  spilled cold. Borrowing was never available: one struct owns both the map and
  the index.

  **The trade, stated rather than buried.** Hot RSS at 50k entities x dim 384
  plus 25k reasons: 510.2 MB → 324.6 MB, −185.6 MB, −36.4%, ten interleaved
  before/after process pairs with 0.5 MB of spread. Query cost, same runs:
  +17 µs median on a ~190 µs index walk, +9%, slower in 11 of 15 pairs. Not the
  refcount — never read on the hot path — and not the indirection; most likely
  locality, since the index's vectors were allocated together during its build
  and now point into the scattered kern map. That mechanism is a hypothesis the
  measurement cannot settle: the host ran at load average 4-6 and the before-arm
  spread was the same order as the effect.

  Recall is unchanged to four decimals (0.9306 / 0.9722 / 0.9471), which is the
  evidence that the two copies were identical and nothing depended on a
  difference between them.

  Supersedes item 29's "halving that needs shared ownership ... across ~20 write
  sites" as unfunded. Item 83 stays open on the half that matters most: nothing
  bounds the resident set, and a smaller O(N) term is a later ceiling, not a
  ceiling.

  Decided by: name-the-tradeoff — the memory win does not get to be reported
  without the latency it cost.

- 2026-07-21 — `4e836bb`'s subject is wrong in the same way `3529fce`'s was, and
  twice makes it a pattern with a clean cause. It reads "source-trust weighting —
  a user-authored claim should outrank an auto-ingested one at equal heat,
  defaulting to 1.0", and every clause of that is what the slice **disproved**:
  user and agent are indistinguishable at scoring time (both mint
  `Source::Inline`), so no `source_trust_user` key was built — grep finds none —
  and what shipped is a scheme-keyed `BTreeMap` that is **empty** by default, not
  1.0.

  The cause is now isolatable. Eight merges reached master today. The two written
  by automation — `3529fce`, `4e836bb` — copy the slice title verbatim and are
  both false. The six written by hand after reading the diff are accurate. The
  difference is not care; it is *input*: a slice title is written before the work
  and describes a hypothesis, and this loop disproves six hypotheses in ten.

  So the fix is not "write better subjects", it is **never derive a merge subject
  from the slice**. Anything that must be generated should come from the branch's
  own commit subjects, which are written after the diff exists. Recorded here
  because a pushed subject cannot be corrected in place, and `git log` is the
  first place anyone looks — someone reading these two lines would conclude kern
  has an importance index and a user-trust knob. It has neither.

  Decided by: verify-before-claiming — both subjects were checked against the
  tree rather than trusted, and both failed.

- 2026-07-21 — filed item 95 while verifying item 20: the file watcher bypasses
  `clamp_confidence` entirely, so an auto-ingested `Document` lands on Beta(2,1)
  = 0.6667, exactly a human CLI claim's posterior and above the 0.6500 an MCP
  agent gets after clamping. A file appearing on disk is trusted more than an
  agent that asserted something on purpose.

  It was found and described by the item-20 slice but left unfiled, inside that
  item's prose. Filing it is the point: item 20's headline is "a user-authored
  claim should outrank an auto-ingested one at equal heat", and this is the
  mechanism that was supposed to make that true. A defect that lives only as a
  sentence inside a neighbouring item is one nobody will ever schedule.

  Deliberately not fixed alongside item 20, because clamping the watcher moves
  the posterior of every `Document` and therefore moves ranking — which item
  20's bit-identity bar forbade. The root-shaped closure is to move the clamp
  inside `Worker::submit` so no caller can skip it, rather than adding a third
  call site each future caller must remember; the guard-every-path-must-pass
  version is exactly what its absence cost here.

  Decided by: fix-the-root — the defect is that the clamp is a convention
  callers follow rather than a gate they cannot avoid.

- 2026-07-21 — source-trust weighting ships as a per-scheme multiplier in
  `apply_boosts`, and the `_user` / `_agent` knobs ROADMAP item 20 asked for do
  not ship at all, because nothing in the tree records who authored a claim.
  `Entity.source` is a `Source` — `{File, Ticket, Session, Agent, Inline}` — a
  URI scheme describing the channel. `kern ingest`, the human path, writes
  `Source::Inline`; the MCP `ingest` tool's default writes `Source::Inline`.
  One tag, two trust principals. A knob named `source_trust_user` could only
  have been keyed on that tag, so it would have read as working and weighted an
  agent identically to a person.

  Confidence does not cover for it either, which was the other way the item
  could have been closed. `clamp_confidence` caps a non-`USER_SOURCE` write at
  `MAX_AI_CONFIDENCE`, worth a 0.667-against-0.650 posterior — a 2.6% edge over
  an MCP agent, and none whatever over the file watcher, which submits `1.0` and
  never passes through the clamp. So the item's headline claim, a user-authored
  claim outranking an auto-ingested one at equal heat, was false in both
  directions it could have been true.

  What shipped is the honest half: `RetrievalConfig::source_trust`, a map keyed
  on `Source::scheme()`, multiplied into the composite post-fusion. Empty by
  default and an absent key is exactly `1.0`, so recall is unmoved — 0.9306 /
  0.9722 / 0.9471, and a bit-identity test on the score words rather than a
  tolerance. `source_trust = { file = 0.8 }` now buys the ranking the item
  wanted, while naming the thing it actually penalises. Unknown keys fail
  `validate` rather than weighting nothing quietly.

  Item 20 stays open on the one thing left: an author principal on `Entity`,
  which is a new field and a store format bump, and belongs with whoever holds
  `src/base/types.rs`.

  Decided by: fix-the-root — a knob keyed on a signal the tree does not record
  is not a smaller version of the feature, it is a label for a distinction
  nothing makes.

- 2026-07-21 — the one cross-slice interaction anyone raised today was checked
  and is a non-issue, and checking it is the point. Item 28's author flagged that
  item 32 rewrote `src/tick/pulse.rs`, where `deposit_pulse` was the path
  enqueueing `TaskKind::Cluster` — one of the three sites that lead to
  `GnnPropagate`. If item 32 changed how often kerns are enqueued for clustering,
  the practical frequency of GNN training changed with it, and neither slice's
  measurements would account for that.

  It did not. `fan_out_cluster` keeps the identical traversal: the same
  `strength < PULSE_THRESHOLD` early return, the same
  `if !k.entities.is_empty()` enqueue, the same `strength * PULSE_DECAY`
  recursion into children. The diff removes only the heat-deposit lines and
  relaxes the graph lock from `&mut` to `&`. Clustering cadence, and therefore
  GNN training cadence, is unchanged.

  Worth recording as its own entry rather than folded into the merge note,
  because it is the first time the parallel structure produced a specific,
  falsifiable question about how two slices compose — the previous entry admits
  the suite is what says they compose and nobody reasons about it. Here someone
  did, named the mechanism, and it took one grep to settle. An agent flagging
  "this lands next to my work and neither of us measured it" is the cheapest
  review this loop has, and it only happened because the author kept looking
  after its own work was committed.

  Decided by: verify-before-claiming — a plausible interaction is not an
  interaction, and the difference is one command.

- 2026-07-21 — merged item 32, the pulse heat deposit removal. 179 + 1 + this
  one = 181, by union rebuild.

  Flagging the merge itself because of what it combines. `cycle/2` closed MCP
  resources against non-public rows and two fail-open holes; `cycle/3` changed
  what the graph forgets. Both are green apart and green together — 902 tests,
  recall exactly 0.9306 / 0.9722 / 0.9471, docs-check clean — but they were
  developed against different masters and neither author saw the other's diff.
  The suite is what says they compose; nobody reasoned about it.

  That is worth stating once, plainly: parallel cycles are verified
  independently and integrated by test, not by design review. It has held all
  day across nine merges, and the anchor checker plus the entry count have each
  caught a class of damage that reading the diff did not. But "the tests pass on
  the union" is a weaker claim than "someone understood the union", and the gap
  is where a subtle interaction would live. Item 32 in particular is a live
  change to retention semantics, not an optimisation: entities in the top five
  levels unread for ~199 days are now collectable where they previously were
  not.

  Decided by: name-the-tradeoff — parallelism bought nine slices in a day and
  costs design-level review of how they compose.

- 2026-07-21 — item 32 `[lifecycle]` closed: the tick pulse deposits no heat.
  Access is now the only deposit, so retention is a function of use and of
  nothing else.

  **The item was right that survival tracked tree position and wrong about which
  way.** Measured first, in `tests/depth_bias.rs` — a chain deeper than the pulse
  can reach, two cohorts at every depth with identical usage, the real `pulse` →
  `commit_access_ids` → `run_gc` lifecycle driven over simulated months by
  rewinding entity stamps rather than sleeping. The reach is **5 levels, not the
  "~4" the item cited from a doc**: 1.0 halved per level stays above the 0.05
  floor through depth 4. Under `relaxed`, an entity read once and never again was
  evicted on day 199.3 at depths 5–7 and **ALIVE at depths 0–4**, carrying heat
  from 1939 (depth 4) to 31 031 (depth 0) against a cold gate of 0.01. Same shape
  on `medium` (day 46.5) and `tight` (day 20.0). So the defect was not that deep
  branches decayed; it was that nothing within four levels of the root could ever
  be collected, used or not — the vision's "the hot graph stays bounded" failing
  outright, on graphs whose whole tree usually fits inside that reach.

  **No deposit size could have been kept.** The deposit recurs every 60s against
  a half-life measured in weeks, so equilibrium is deposit/(1−decay-per-tick): any
  amount above ~1.6e-7 settles above the 0.01 gate and grants the same permanent
  exemption. That rules out tuning it, and it rules out the fix the item's title
  implies — propagating *deeper* would have extended the exemption to every entity
  in the graph and disarmed cold GC completely. Removing the deposit was the only
  option that leaves heat meaning what `VISION.md` says it means.

  **Stated cost.** Eviction changes: entities in the top five levels that go
  unread past 6.64 half-lives (~199 days on the default `relaxed`, ~46 on
  `medium`, ~20 on `tight`) are now collected where before they never could be.
  They spill to the cold tier first and recall backfills from it, active Facts and
  Documents keep their immunity, so nothing durable is lost — but a long-idle
  graph will shed hot rows it used to keep, and that is the point. Gossip's
  `hottest_local` batch changes with it, from "whatever sits near the root" to
  what has actually been read. Recall does not move: heat feeds GC and gossip
  selection, never ranking, and `e2e` re-measured at exactly 0.9306 / 0.9722 /
  0.9471. The tick got cheaper rather than dearer — the deposit was O(entities in
  reach) heat writes plus a `Vec<String>` of every entity id per kern, under the
  graph **write** lock; over 342 kerns / 102 300 entities the same pulse fell from
  **17 586µs to 341µs**, and now takes a read lock.

  Proven by revert: restoring the deposit inside the walk fails
  `tick::pulse::tests::at_equal_usage_survival_does_not_depend_on_depth` with
  "depth 0: a thought untouched for 9 days survived while the identical thought at
  depth 7 was collected — survival is tracking tree position, not usage". The test
  is 8 kerns deep and 9 simulated days long on purpose: a tree inside the reach,
  or a horizon under `COLD_GC_AGE`, passes whether the bias exists or not.
  `HeatConfig::deposit_traversal` is deleted rather than left at 0.0, and
  `pulse_with_heat` collapses back into `pulse`, since a heat argument that
  deposits nothing is a lie the next reader has to disprove. Docs corrected in the
  same change: `stigmergy.mdx` called the deposit "load-bearing" and claimed a
  subtree near the root "survives GC more readily" — it survived absolutely —
  and `stigmergy-over-gardening.mdx` had already written down "pulse-reachable
  never qualifies" without anyone reading it as the defect it was.

  Decided by: fix-the-root

- 2026-07-21 — item 18 `[surface]`, the separable half: the MCP **resources**
  surface is default-deny. `resource_thoughts`, `resource_thought` and
  `resource_reason` returned entity text to any client that could open the
  transport with no ACL consulted; `resource_reason` checked nothing at all,
  which made it a read of scoped entity text through an id that is not the
  entity's, since `link`'s `explain_relationship_prompt` writes edge text from
  both endpoint texts. `Acl::is_public()` now lives on `Acl` and `acl_admits`
  calls it, so "public" has one definition shared with `matches_filter` instead
  of two copies that could drift.

  **Why this is separable from item 24.** Item 24 is the missing-auth boundary —
  how a principal *arrives*, per-read or per-session. That question is untouched
  here and stays open. The separable question is what a surface that can name no
  principal may return *at all*, and it has an answer that does not depend on the
  other: only rows carrying no ACL. Default-deny is the floor any principal
  scheme can only widen, never contradict, so landing it early cannot constrain
  item 24's design.

  **Decided by: an edge endpoint that does not resolve gets its own verdict —
  the edge is served, its text is not.** This was the load-bearing call and the
  first cut got it wrong twice. `find_entity` (`src/base/search.rs:148`) walks
  only the *resident* kern map: `loaded` is `kerns.get`, `all()` is
  `kerns.values()`, and neither sees `unloaded` or the cold tier. So "did not
  resolve" is **not** "does not exist" — a GC cold-spill
  (`src/tick/stigmergy.rs`) or a kern-cap unload (`GraphGnn::unload`) leaves a
  scoped row alive in the store with its ACL intact and invisible to this
  surface, while the edge quoting it survives untouched, because a kern hosts a
  reason iff it hosts its `from` (`src/base/reason.rs:78`): `move_entity` leaves
  an incoming edge in the *source* kern and `remove_entity` cascades only within
  one kern. Reading unresolved as "allowed" would therefore let a scoped row
  become readable **by disappearing**, with no race in it — that is the stable
  committed post-GC state, not a window a wider lock would close.

  Failing closed on it was not available: a dangling endpoint is ordinary here
  (`Reason::to` is optional in `add_reason`), and dropping every edge with one
  hides a public entity's own structure — default-deny becoming deny-all, which
  is the failure the four pre-existing tests exist to catch and which they do
  catch. So the verdict is three-valued (`Endpoint`): **Scoped** drops the edge,
  **Public** serves it whole, **Unresolved** serves the edge with its `text`
  withheld. The text is what carries the disclosure — up to 500 chars of both
  endpoints via `explain_relationship_prompt` — and the ids that remain are
  `content_hash(text)`, which at worst confirms a guessed text. `resource_reason`
  runs the same rule with one asymmetry: `from` fails closed on Unresolved too,
  because it is the entity the edge hangs off and one that did not resolve is not
  one that said the read was allowed.

  Also decided: **both** ends of an edge are gated, not just `from`. Gating
  `from` alone still served `reason://{id}` for an edge whose `to` was scoped,
  naming the scoped id outright beside text written from it.

  Not closed, and named rather than waved off: `kern://local/health` and
  `kern://local/kerns` count every entity and reason, scoped included
  (`graph_health_stats`, `src/base/health.rs:48-54`), so a scoped ingest moves a
  number. No ids, no text, and the same count the operational `health` tool
  reports — narrowing it is the separate question of what an unauthenticated
  operational surface may say. Gossip egress still replicates scoped entities
  ungated, and `Reason` still carries no ACL of its own, so every reader
  re-derives the verdict from the endpoints; storing it at write time is the real
  fix. Item 18 stays open on those and on principal arrival.

  Seven unit tests, one per guard: each guard mutated away fails exactly its own
  test and no other. The four pre-existing resources tests are byte-identical.
  Baseline 891 → 898.

- 2026-07-21 — merged item 29, which closed unbuilt and shipped a different fix.
  176 + 1 + this one = 178, by the union rebuild rather than a hand-spliced
  conflict hunk.

  The pattern across today is now clear enough to state as a finding rather than
  an anecdote. **Nine slices ran; six ended somewhere other than their title
  pointed.** Item 25 asked for an index twice and got neither — first because the
  scan did not dominate, then because it had no sound freshness signal and the
  real defect was that it was never parallel within a kern. Item 26's premise was
  false: PageRank is query-personalised, so the caching it prescribed was the
  rejected design. Item 27's first bullet was 0.06% of the sweep and its second
  was real for a different reason (bincode decode, not cosine). Item 28's panic
  clause cited an item that had already closed. Item 29's premise held but its
  remedy made memory *worse* by 122 MB, and chasing the variance found a build
  that was not reproducible.

  What made the difference every time was the same rule: measure before
  implementing, and treat the item as a hypothesis. The two items that shipped
  what they asked for — 28 and 30 — are the two whose measurements confirmed
  them. That is the loop working, not failing: an item is a hypothesis written
  down by someone who could not run the experiment yet.

  The corollary is uncomfortable and worth keeping: **a slice title is the least
  reliable sentence in the file**, and any automation that names a merge after
  the slice rather than the diff — as `3529fce` did — will be wrong precisely on
  the passes where the work was most useful.

  Decided by: verify-before-claiming — six of nine.

- 2026-07-21 — item 29 closed with no index-spilling code, and the DiskANN build
  made reproducible instead. The item said a spilled kern still carries two
  resident indexes. It does — `rebuild_index` hardcodes `gnn_entity_idx` and
  `reason_idx` resident (`src/base/graph.rs:289-290`) — so for once the premise
  held. What had never been checked is whether spilling them would help.

  Measured with `tests/spill_memory.rs`, one process per configuration because
  glibc does not return HNSW's many ~1.5 KB vector allocations, and RSS read
  twice — cold, and hot after 200 searches, since mmap pages are only resident
  once touched. At 50k entities, dim 384: all-resident 510.3 MB hot; entity
  spilled 512.1 MB hot; **all three spilled 632.3 MB hot.** Doing what the item
  asked costs 122 MB. Spilling the entity index alone looks like it saves 71.4 MB
  and gives every byte back the moment a query touches the snapshot.

  So the trade that spill actually makes is not the one its decision doc claimed
  ("drops heap by the full vector set"): it converts ~97 MB of unreclaimable
  anonymous heap into clean file-backed page cache. Worth having under memory
  pressure, and not a ceiling moving. `diskann-spill.mdx` and
  `docs/kern/diskann-disk-index.md` were corrected in the same change.

  The largest resident holder turned out to be no index at all — the kern map is
  260.7 MB of the 512.1, because every vector is stored twice, once in
  `Kern::entities` and once in the index pointing at it. Spill relocates one
  copy. Removing one needs shared ownership across ~20 write sites, so it is
  named in item 83 rather than attempted here.

  **What did ship** is the defect the measurement exposed: `build_and_save` was
  not reproducible, despite a seeded RNG and a comment asserting it was. Two
  hashed containers reach the adjacency and the `sort_by` ranking candidates is
  stable, so every tied cosine distance broke in per-process hash order — the
  same corpus built a different index in every process. Only a corpus with tied
  distances can detect this, which is why the first version of the guard passed
  against dense random floats and proved nothing. Two `BTreeSet`s
  (`src/base/diskann.rs:123`, `:180`); reverting the first differs by
  22740/76800 adjacency bytes, the second by 446/76800, and reverting
  `greedy`'s visited list by none — so that one was not changed.

  **The tradeoff, named:** reproducibility here costs a `BTreeSet` insert where a
  `HashSet` insert was, on sets of ≤ 64 ids, against a per-candidate cosine over
  384 dimensions — unmeasurable, and not measured. The real cost is that
  `tests/spill_transparency.rs` now records what spill costs in recall and it is
  not zero: 1.0000 resident vs 0.9940 spilled against brute force, 0.9940
  overlap. Spill is **not** answer-preserving. It cannot be — it swaps HNSW for
  Vamana — so the round-trip test asserts recorded floors, not equality, and
  saying otherwise anywhere in the docs would have been the easier lie.
  `e2e` recall unchanged at 0.9306 / 0.9722 / 0.9471; the default path never
  spills, so nothing here reaches it.

  Decided by: verify-before-claiming — the item was tested as a hypothesis, and
  the remedy it prescribed was priced before being refused.

- 2026-07-21 — item 28 `[lifecycle]` closed: GNN training runs on its own thread,
  not on the tick loop. Unlike the last three perf items, this premise survived
  measurement. A new release-only instrument (`tests/gnn_scale.rs`, in the shape
  of `tests/gc_scale.rs`) put one propagation at 0.64s at 128 entities — the
  smallest kern that trains at all, `min_thoughts` — 6.4s at 1024, 21.6s at 2048
  and 79.7s at 4096, against `stigmergy_gc` at 0.151ms, `commit_access` at
  0.002ms and `idle_sweep` at 0.000ms on the same graphs. The stall was then
  measured on the real loop rather than argued from the source: the recall path's
  own heat write-back, the `CommitAccess` task an MCP query enqueues, landed in
  2.2ms with nothing ahead of it and in **56 787 ms** with one propagation ahead
  of it at 2048 entities. After the change, 1.2ms.

  **The overlap policy is coalesce-then-refuse, and the coalescing is the
  interesting half.** A second propagation request for a kern already waiting is
  folded into the waiting one rather than queued, because the propagation
  snapshots the graph when it *runs*, not when it is enqueued — so the job
  already in line will train on everything the newer request would have seen.
  Only genuinely different kerns can queue, eight of them, and past that the
  newest is refused and counted, which is the shape item 30 settled on for the
  ingest queue. Rejected: `spawn_blocking`, whose 512-wide pool would train every
  kern at once while each training allocates a dense N×N adjacency — 134 MB at
  4096 entities alone; and an unbounded queue, which is precisely the growth
  defect item 30 had just closed elsewhere.

  **The panic story is relocation, not improvement, and saying otherwise would
  have been the easy lie.** Item 2 is closed, so the inline arm was already
  contained by `run_guarded`; moving the work moves that containment rather than
  adding it. A bare thread would have been strictly *worse* — the first panicking
  propagation kills the trainer and every later one silently never runs. The
  revert test proves exactly that: with the trainer's `catch_unwind` removed the
  `kern-gnn` thread dies and the panic count stays 0. It is kept, and it records
  through the same `Queue::record_task_panic` the health surfaces already read,
  so `GnnPropagate` remains the one task that reports a contained failure.

  Moving training off the loop opened one race the inline version could not have,
  and it is fixed in the same change: an entity superseded *during* training
  would have been re-inserted into `gnn_entity_idx` by the apply step, undoing
  the supersede removal, so `apply_gnn_updates` now re-checks status at write
  time as well as at snapshot time.

  `pytest -q -s e2e` returns 0.9306 / 0.9722 / 0.9471, unchanged — which is what
  says the propagation still produces what ranking reads. What is left, and the
  reason the item is closed rather than deleted, is the cost itself: 79.7s at
  4096 is untouched, because `normalized_adjacency` materialises a dense N×N
  matrix over a graph ingest keeps sparse at roughly one similarity edge per
  entity. A sparse adjacency would make training linear in edges instead of
  quadratic in nodes, but it changes the numerics ranking reads and so needs its
  own recall gate rather than a ride on this one. Also left: `gnn_train_refused`
  reaches MCP health only, not `kern status` or the RPC `HealthRes`.

  Decided by: verify-before-claiming

- 2026-07-21 — item 94's fix simplifies to "delete the file". Cargo's default
  `target-dir` is `<workspace-root>/target`, which inside a git worktree is that
  worktree's own directory, so isolation is what you get by doing nothing.
  Confirmed rather than assumed: `cycle/2` has no `.cargo/` at all and resolves
  to `kern-cycles/2/target` unprompted.

  Which means the entire contamination defect was self-inflicted — introduced by
  a launch step that wrote a `.cargo/config.toml` pointing every worktree at the
  main checkout's cache, to buy a warm build. Pointing that file at a per-tree
  path also works, and is what three live trees currently do, but it is a second
  thing to keep correct forever. Not writing the file has no failure mode.

  Worth recording as its own entry because the first correction to item 94 fixed
  the diagnosis and left the remedy one step short: it said "give each worktree
  its own target-dir", which is true but reads as "configure something", when
  the actual instruction is "stop configuring something". A fix that requires
  ongoing correctness is worse than one that requires nothing, and the launch
  step that caused this is exactly the kind that gets copied forward unexamined.

  Decided by: fix-the-root — the root was the config file's existence, not its
  contents.

- 2026-07-21 — `3529fce`'s subject says "item 25, the importance scan indexed"
  and **no index was built**. The merge subject was generated from the slice
  title — the work that was *asked for* — rather than from the diff, which
  refused the index on evidence and parallelised the inner loop instead. Checked
  before writing this: no `importance_index` symbol exists anywhere in `src/`,
  and `an_eligibility_change_is_reflected_with_no_epoch_bump` — the test that
  exists specifically to fail if someone memoises this scan — is present and
  green.

  Recorded because `git log` is the first place anyone looks and that subject is
  a lie about the tree. `ROADMAP.md` item 25 and the entry below it are both
  accurate; only the commit subject is wrong, and a commit subject cannot be
  corrected in place once it is pushed. So the correction lives here, where the
  next reader of the history will also be looking.

  The general shape is worth naming: **a slice title is a hypothesis, and this
  loop keeps disproving them.** Four perf items today ended somewhere other than
  where their title pointed — 25 twice, 26's premise, 27's first bullet. Any
  automation that names a merge after the slice rather than the diff will
  therefore be wrong precisely on the passes where the work was most useful.

  Decided by: verify-before-claiming — the subject asserted an index, the tree
  was checked for one, and it is not there.

- 2026-07-21 — item 25 narrowed, and the index it asked for deliberately not
  built. Re-measuring after item 26 confirmed the scan now dominates every
  eligibility level — 39.7 / 60.1 / 72.1 / 71.2% of retrieve at N=100k, against
  a PageRank that item 26 cut from a flat ~20 ms to ~2–3 ms — so the item was
  live and the old "PageRank first" table it carried is replaced.

  Then the index turned out to have no sound freshness signal.
  `bump_mutation_epoch` has three callers, all inside `graph.rs`, while
  `GraphGnn::kerns` and `Kern::entities` are public and ~20 non-test sites write
  through them directly. The decisive one is `commit_access_ids`, which stamps
  access on every delivered result and bypasses `get_mut` *on purpose* so it will
  not invalidate the semantic query cache — so the single mutation that creates
  importance is exactly the one an epoch-keyed index can never see. Proven, not
  argued: a `mutation_epoch`-keyed memo over `seed_important` makes the new
  `an_eligibility_change_is_reflected_with_no_epoch_bump` fail on the access
  crossing. That test ships as the guard against the next attempt.

  What did ship is the root the measurement exposed: the scan was never parallel
  on the corpus everyone actually has. `par_iter().flat_map_iter(...)` split over
  *kerns* and walked each kern's entities on one thread, so a single-kern graph
  scanned serially on 8 idle cores — and this item had described itself as
  "rayon-parallel" throughout. Splitting the inner walk gives 1.9–3.9× at N=100k
  (35.7 -> 12.0 ms at full eligibility) with recall bit-for-bit unchanged at
  0.9306 / 0.9722 / 0.9471, and the selection proven bit-identical against an
  independently written sequential gate rather than merely equivalent.

  **The tradeoff, named:** this buys nothing at N=10k, where the numbers sit
  inside the noise of a box running three worktrees. It is a large-corpus fix
  that leaves the O(N) walk intact — the cliff is postponed, not removed, and
  anyone reading "faster" here should read "still linear".

  Also found, and worth someone's attention: the shared `target-dir` **does**
  serve artifacts across worktrees, which item 94 explicitly ruled out. A clean
  tree at 5d0a2bc failed to compile against a `HealthRes` field that does not
  exist in its own source — slot 2's in-flight `ingest_queue_refused` — because
  `libtrnsprt-*.rmeta` is last-writer-wins between trees. `touch`ing the local
  `dto.rs` fixes it; a sibling rebuild brings it back. Every build in this pass
  needed that touch first. Not filed here, because this pass owns one item.

  174 entries.

  Decided by: fix-the-root — the item asked for an index, but the root was that
  entity mutation is unobservable and that the scan was not parallel at all;
  building the index on an epoch that cannot see the mutation would have shipped
  a silent recall bug instead of fixing either.

- 2026-07-21 — item 27 `[lifecycle]` narrowed to one bullet it never contained.
  Both remaining claims were measured first (`tests/gc_scale.rs`, release, new
  here alongside `tests/seed_scale.rs`). **Victim selection does not dominate and
  was not touched**: 3.8 ms at 100k entities against a 6 045 ms sweep, 0.06% of
  it, and linear rather than superlinear — one predicate per entity once per GC
  interval. No index was written, and none would have worked: eligibility is
  decayed heat, a function of `now` and each entity's own `heat_updated_at`, so
  no static ordering survives the clock advancing. That is the second perf claim
  in this file withdrawn by its own instrument, after item 25.

  **The cold-tier scan did dominate, but not for the stated reason, and the fix
  is not an index.** `cold_search` cost 470 ms per call at the 50k cap, on the
  *recall* path — it fires whenever the hot tier underfills `k`. 87–99% of that
  was bincode-decoding a whole `Entity` per row to reach a vector, not the cosine
  arithmetic: `cold_all`, the same decode with no scoring, cost 343 ms of the
  393 ms. So vectors moved out of the row into their own LMDB table; the scan
  scores off raw little-endian floats and decodes only the k winners. 470 ms →
  28 ms at 50k rows, 72 → 6 ms at 10k. Still a full scan, deliberately: the cold
  tier exists to *not* be resident, and an ANN over it would put the index back.

  **What it costs.** One extra `put` per spill, inside the transaction that was
  already being committed, so no added commit and no added fsync — across three
  paired release runs the per-spill time difference stayed inside run-to-run
  noise (2.3–9.8 ms either way), which is to say it is not measurable. The cost
  is disk: the side table holds vectors uncompressed where the row zstd'd them.
  At 5 000 spills of 384-dim *dense* vectors — what an embedding model returns —
  20.63 MB → 21.34 MB, +3.4%. On a highly compressible vector distribution it is
  far worse: the sparse fixture went 0.95 MB → 21.34 MB, 22×. Memory moved the
  other way: a search now buffers 50k `(id, score)` pairs instead of 50k decoded
  entities, tens of MB less peak. The store format is bumped to `FORMAT_V6`
  rather than migrated, per the alpha no-compatibility rule; old stores are
  rejected cleanly and reingested.

  Selection equivalence is pinned by
  `cold_search_selects_exactly_what_the_linear_scan_did`, which compares the new
  path against the old linear scan over a 600-row generated tier — ids, scores
  *and* vectors, across 8 queries × 5 values of k, with wrong-dimension, empty
  and deliberately tied vectors in the fixture. Comparing vectors is not
  decoration: an earlier version compared only ids and scores and passed with the
  vector join deleted. `pytest -q -s e2e` returns 0.9306 / 0.9722 / 0.9471,
  unchanged, which is what says GC still forgets exactly what it forgot before.

  Supersedes item 27's title and its two open bullets. What is left is the cost
  the measurement actually found and the item never listed: `cold_spill` opens
  and commits one LMDB write transaction per victim, 3–10 ms each, 279 s for a
  80 000-victim sweep. Batching it is easy — `cold_put_all` already does — and is
  left open on purpose, because one transaction for the whole list turns
  per-victim spill-before-drop into all-or-nothing, and that is a retention
  decision rather than a performance one.

  Decided by: verify-before-claiming

- 2026-07-21 — item 94 corrected: the shared `target-dir` **does** leak between
  worktrees, and the earlier entry saying it does not was wrong. That entry
  checked the wrong artifact class. Lib-test binaries really do get one hash per
  tree — four hashes, four trees, verified. But a workspace sub-crate has the
  same package name, version and relative path everywhere, so `trnsprt`'s
  fingerprint matches across trees and cargo reuses whichever build landed first.

  Two independent observations forced it. `cycle/3` failed to compile against a
  field that existed only in `cycle/2`'s source. `cycle/2` watched cargo report
  `Fresh trnsprt` against a stale rmeta with hard-link count 3 while its own
  `dto.rs` edit sat on disk, and had to `touch` that file before every build it
  measured from.

  The second mode is the dangerous one and it is the one the earlier entry ruled
  out: a tree linking a sibling's crate. It failed loudly here only because the
  two sources disagreed about a struct field. A type-compatible divergence would
  have linked silently and produced a green suite against another branch's code —
  which is the whole verification story of this loop, wrong.

  All three worktrees now have their own `target-dir`. ~33 GB against an 11 GB
  shared cache, and both stalled builds recovered on the spot. The
  staleness-guard alternative is moot: it addressed mode 1 and was blind to
  mode 2. The by-name test discipline survives, because it catches mode 1
  independently of the build layout and is what confirmed both cycles.

  Decided by: verify-before-claiming — the first version of item 94 asserted
  "checked rather than assumed" about a check that examined the wrong artifact,
  and the correction cost two stalled cycles to surface.

- 2026-07-21 — bounded the ingest queue, and answered the question item 30
  existed to force: **when the queue is full, refuse the newest job — except on
  the one leg that can be slowed instead, which waits.**

  First the premise, because the item offered two defects and did not say which:
  it grew, it never dropped. `tokio`'s `send` errors only on a closed channel,
  so each detached `tokio::spawn` parked on the full queue still holding its
  whole text. Measured, not reasoned: 500 jobs offered to a worker stalled on a
  hanging embedder were all 500 accepted. That number is now the revert-check
  failure of the test that bounds it.

  Refuse-newest wins because the three alternatives each pay somewhere worse.
  *Block the producer*: correct, and it welds ingest throughput to the slowest
  thing in the system — the MCP `ingest` tool would sit on embed latency while an
  agent waits. *Drop the oldest*: discards work already accepted, and the RAM
  queue is reached exactly when the durable file intake is unavailable, so the
  discarded job has no disk copy to recover from. *Drop the newest silently*:
  same loss, and the caller is told "queued". Refusing keeps the loss where the
  caller can see and retry it; the cost is honest — `enqueue` now returns
  `Option<String>` and the MCP tool can fail where it never could.

  The file watcher is the exception and gets the waiting form, `submit`. It is
  the fast producer the bound exists for, nothing is waiting on its return, and
  its own backlog is coalesced paths rather than job bodies — so stalling it
  costs less memory than the pileup it replaces, while a refusal there would
  lose a file nothing re-offers.

  Refusals are counted and surfaced as `ingest_queue_refused` on all three health
  surfaces, following `unspilled_drops` rather than inventing a shape, with the
  warn line throttled and the counter exact. A test walks a real refusal from the
  worker's counter to the RPC DTO an operator polls; it fails when the RPC leg is
  hardcoded to zero.

  Recall unmoved at 0.9306 / 0.9722 / 0.9471, as it must be — the CLI and intake
  legs use `run`, which awaits and never touched this path.

  Decided by: name-the-tradeoff — four defensible policies, so the entry records
  what each rejected one would have cost rather than letting the code imply it.

- 2026-07-21 — merged item 26's confined PageRank, and resolved the `CHANGELOG`
  collision by rebuilding the union rather than splicing the conflict hunk. Last
  time the splice silently dropped four entries that lived outside the hunk on
  screen, and reading the diff did not reveal it — only Ruling 1's before/after
  count did. So the method is now: take one parent's file whole, prepend only the
  entries the other parent has that the merge base does not, and state the
  arithmetic. 166 + 2 + this one = 169.

  Worth writing down because it is the second time a merge has been the thing
  that damaged the record rather than the work. A conflict in an append-only log
  is not really a conflict — both sides appended, nothing disagreed — but the
  three-way markers present it as one, and hand-editing inside them is where
  entries go missing. The union is computable, so it should be computed.

  What landed: the power iteration confined to the reachable set, bit-identical
  by the `+0.0` argument, 18.8 ms -> 1.7 ms at 1% eligibility with recall exactly
  unchanged; item 26 left open on the 1.4x regression at full reach; and item 94,
  the shared `target-dir` reporting green on stale code. `docs-check` green at
  691 references with no anchors nominated — the third consecutive merge to break
  no citations.

  Decided by: fix-the-root — the recurring damage was the resolution method, not
  the conflicts, so the method changed.

- 2026-07-21 — filed item 94: the shared `target-dir` behind the parallel cycles
  can report green on stale code. A cycle saw `873 passed` with its own three
  new tests absent from that run; `touch src/lib.rs` brought the count to a
  self-consistent 867. Checked before believing the scarier version: this is
  NOT cross-worktree contamination — four trees produce four distinct binary
  hashes, and `across 7 binaries` is the normal count for a four-package
  workspace with integration tests. The main checkout reporting 872 where a
  worktree reports 865 is different code, not a mixed run.

  The rule this leaves behind is the useful part: **an aggregate pass count is
  not evidence that a new test ran.** Verifying this cycle used the discipline
  the item now records — `-E 'test(pagerank)'`, ten tests named and ten results
  read, which a stale binary containing none of them cannot satisfy.

  Not fixed here. Per-worktree target dirs are correct by construction and cost
  a cold build each plus ~33 GB; a staleness guard keeps the cache the sharing
  was adopted for. Flipping the build layout under three live cycles would stall
  all three, and one incident is not yet a trend.

  Decided by: verify-before-claiming — the reported symptom was taken seriously
  and the reported cause was not; the cause turned out narrower than claimed,
  and the narrower version is the one worth defending against.

- 2026-07-21 — item 26 asked for a cached PageRank vector recomputed on a tick,
  on the stated grounds that "the scores depend on the graph, not on the query".
  They depend on the query. `fuse_hybrid_seeds` personalises the teleport vector
  at the query's dense and lexical seeds, so a per-graph vector is the *global*
  PageRank that `decisions/pagerank-authority.mdx` already weighed and rejected —
  "popular entities top every query, relevant or not". Caching it would not have
  been a faster version of this feature; it would have been the alternative the
  feature exists to avoid, arriving as a performance change with the ranking
  regression buried inside it.

  So the premise was checked before the work was done, and the closure changed.
  What ships instead is exact: the power iteration is confined to the teleport
  support and everything downstream of it. Every node outside that set holds a
  literal `0.0` in both rank vectors, so every term the full-width loop added for
  it was `+0.0` and every term it summed was unchanged — walking the reached set
  in ascending order leaves the surviving terms in the same order, which makes
  the result identical rather than close. The test compares bit patterns against
  a verbatim copy of the loop that was replaced, over 108 configurations of
  graph, seed set, `top_k` and iteration cap. e2e recall is unmoved to four
  decimals: 0.9306 / 0.9722 / 0.9471.

  Measured, `tests/seed_scale.rs` in release at N=100k: a flat 18.8 / 17.9 /
  18.1 / 17.1 ms across 1 / 10 / 50 / 100% eligibility became 1.7 / 3.1 / 1.9 /
  1.3 ms, and a filtered retrieve went 24.0 ms to 7.1 ms. Three post-change runs
  spread those points over 1.3–6.0 ms, so the band is what is claimed and not
  any single point in it; the before figures sat at 17–19 ms, well outside it.
  The cost now tracks
  reach rather than N, which is a different shape, not a smaller version of the
  same one — so the tradeoff is stated where it can be found: at full reach the
  confined walk is **1.4× slower** than what it replaced, because it indexes
  through a list where the full-width loops vectorise over a slice. That is
  measured too, by an instrument kept next to the code rather than described in
  prose, and item 26 stays open on exactly it.

  Decided by: avoided-question-first — the item named its own answer, the
  avoided question was whether that answer was compatible with the ranking it
  was not allowed to move, and it was not.

- 2026-07-21 — `Acl` stopped being decorative. It has been on `Entity` since the
  beginning and was only ever written as `Acl::default()`, so no caller could
  populate it and no reader consulted it. Item 18's three remaining bullets
  closed together, because separately each is inert: `ingest` takes `scope` +
  `principals` and `acl_from_args` builds the `Acl` on the same pre-branch line
  as `valid_until`, so the sync, durable-direct and RAM-queue paths carry it
  identically; it rides `ingest::Job::acl` into `new_statement_entity`, which no
  longer hardcodes a default; `query` takes `principals` into
  `QueryOptions.principals`; and `acl_admits` runs first in `matches_filter`.
  One predicate, so the ranked read and the id read got the rule at once — the
  id path having been routed through `matches_filter` earlier the same day.

  **Two rules the item states, both enforced and both pinned by a test that
  fails if you delete the rule.** A scoped **`Fact` is dropped for a
  non-member** — GC-immunity is not ACL-immunity, and nothing in the predicate
  looks at `kind`. And an **empty `principals` is no filter, not public-only**:
  a caller who names no principal still reads everything, which is the only
  reason `kern get` and every existing unscoped read still work.
  `matches_filter_is_the_per_entity_predicate` asserts the scoped-Fact drop, the
  member keep and the empty-principals keep; `bare_id_read_still_serves_a_scoped_row`
  asserts the second rule again on the id surface, where getting it wrong would
  read as data loss rather than as a policy.

  A blank principal is a hard error (`parse_principals`, shared by both
  surfaces), not a silent skip — it would match the empty `Acl::scope` of every
  public entity, so accepting it would turn a typo into an access decision.
  `DirectJob::acl` carries the ACL across the durable intake hop for the same
  reason `valid_until` is carried; without it the async path would republish a
  scoped ingest as public. `src/tick/tasks.rs` carries the old entity's `Acl`
  into a rephrase, so a supersede cannot launder a scoped thought into a public
  one. `Worker::enqueue`/`run` keep their arity and delegate to the `_with_acl`
  forms with `Acl::default()`, which is what deliberately leaves the file
  watcher and the intake drain public — item 18's own recommended default, and
  its one still-open decision.

  **Say the coverage plainly: this is unit-tested only, and cannot be more.**
  `principals` is MCP-only, there is no CLI flag, and `e2e/conftest.py` drives
  the binary over subprocess with no MCP JSON-RPC client — so no e2e test can
  reach this surface without an MCP stdio driver fixture larger than the
  feature. `ingest_acl_tests` is the compensating end-to-end: it drives the real
  `ingest` tool against a stub embedder and reads the `Acl` off the entity that
  landed in the graph, so the chain is proven, not the schema.

  **The dedup question is decided, because leaving it open left a leak.** On
  dedup the survivor keeps its own ACL — an id *is* its content hash, so one
  text cannot exist under two audiences and there is no other answer available.
  That makes the `Rephrase` edge the hole, not the entity: `merge_duplicate`
  stored the incoming text **verbatim** on the survivor, a `Reason` carries no
  ACL of its own, and every surface that renders an entity renders its edges. A
  scoped ingest landing within `dedup_threshold` cosine of any public thought
  published its own text to everyone. `merge_duplicate` now takes the incoming
  `Acl` and skips the rephrase write when it differs from the survivor's, in
  either direction; corroboration is metadata about a statement and still
  merges, the wording does not. `a_scoped_dedup_does_not_write_its_text_onto_a_public_survivor`.

  Two more reads reached entity text without passing the predicate, found by
  enumerating the read surfaces rather than trusting the one gate.
  **Cold-tier backfill** (`src/mcp/tools_query.rs`) pushed `Store::cold_search`
  hits straight into the delivered set — a raw cosine scan that answers no
  filter, so spilling an entity was the way around every predicate the hot path
  enforces. **Path chains** (`format_chains`) render the text of every entity on
  a walk, and `retrieve` filtered only `results`; the ACL stopped the row and the
  chain printed it anyway. Both now run `matches_filter`; a chain touching a
  withheld entity is dropped whole, since a chain with a hole still says the
  withheld thought exists and what it connects.

  Still not gated, and named rather than fixed: the MCP **resources** surface
  (`kern://thoughts`, `kern://thought/{id}`) returns entity text and takes no
  `principals` at all, so the ACL is a filter a cooperating client asks for, not
  a boundary imposed on one; and **gossip egress** replicates a scoped `Entity`
  to peers ungated (the `Acl` does ride the wire, and `merge_entity` never
  imports a remote ACL over a local one, so neither side can widen the other's).
  Both are recorded on item 18, which stays open.

  Decided by: verify-before-claiming — the handed-down mutation table was
  re-run rather than quoted (all three rules do fail loudly), and the baseline
  was re-measured at 866, not the number passed down. fix-the-root — one gate
  that holds is worth less than knowing which reads run it, so the surfaces were
  enumerated and the three that bypassed it are the actual defect.

- 2026-07-21 — item 89 `[ingest]` closed: retention reaches the two entrances
  that have no caller to pass a flag, and it has a `kern.toml` home the item
  itself pointed at the wrong section for. A `.txt` transcript and a watched
  file are dropped by nobody in particular — there is no call site to hand a
  `--retention-secs`, so their TTL can only be a standing policy. Neither had
  one: `drain_entry` cloned the queue's `Config` and overwrote only
  `valid_from`, and `KernFileWatcherSink` handed the worker
  `IngestRunConfig::default()` outright, two hardcoded no-TTLs. Both now read
  `Config::with_retention`, which delegates to the existing
  `valid_until_from_retention`, so there is still exactly one duration→instant
  conversion and the four entrances cannot drift.

  **The key could not go where the item said to put it, and that is the whole
  decision.** Item 89 named `IngestConfig` — `[ingest]`. But
  `Config::load_with_user` refuses a user-written `[heat]`, `[ingest]` or
  `[retrieval]` section outright (`src/config/mod.rs:121-126`): those are
  preset-owned and `Preset::apply` is their only writer. A `retention_secs`
  beside `dedup_threshold` would therefore have been settable by no `kern.toml`
  in existence, and the half the item itself called load-bearing would have
  shipped dead on arrival. It went to `[intake]` and `[watcher]` instead —
  per-source retention is keyed by *source*, not a preset dial, and those two
  sections are exactly the ones that name the sources and that a user is allowed
  to write. `a_real_kern_toml_can_set_per_source_retention` proves it by
  loading a real file through `load_with_user`, not by constructing the struct.
  `src/config/mod.rs` also now actually calls `watcher.validate()`, which it
  never did; both sections refuse an unrepresentable retention at boot rather
  than logging it once per drain pass forever.

  **Where `with_retention` is called matters more than what it returns.** The
  deadline is an absolute instant, so a config built once above the intake poll
  loop — where it lived — would hand every transcript a deadline measured from
  daemon boot, expiring a month-old delta and a minute-old one at the same
  moment. The config moved inside the loop; the sink resolves per record. Proven
  by reverting each half, not by the tests passing: hoisting the config back
  above the loop collapses two transcripts queued two seconds apart onto a
  single deadline and fails
  `the_poll_loop_resolves_its_deadline_per_pass_not_once_at_startup` alone, and
  reverting `kern intake drain`'s wiring to a literal `0` fails the new e2e with
  "still delivered well past its retention deadline" while its no-policy control
  stays green.

  Named, not closed over: the file-watcher half is **unit-covered only**.
  `WatcherConfig::enabled` is `false` by default and nothing in `e2e/` starts a
  watcher, so `the_sink_stamps_the_configured_retention_on_what_it_ingests` is
  the entire proof for that entrance. A durable `direct/` job also still cannot
  inherit a standing policy — `drain_direct_once` overlays `job.valid_until`
  over the loop's config, and an absent flag and `--retention-secs 0` are
  indistinguishable on the wire. 868 tests pass against a true baseline of 862
  (not the 866-with-a-skip this cycle was briefed with; there is no `#[ignore]`
  in the tree), and the e2e floors are unmoved at 0.9306 / 0.9722 / 0.9471 —
  which for a 36-fact corpus is a statement that nothing regressed, not a
  measurement of this change. Also removed: a stray diff3 `|||||||` conflict
  marker that this branch's own merge (`8ecd15f`) left in this file. Master
  never had it.
  Decided by: fix-the-root — the item's literal instruction was to add a key to
  `IngestConfig`, and obeying it would have produced a config key that no config
  file on earth can set. The root is *which sections a user may write*, so the
  key went to the sections that describe the sources rather than to the one the
  item happened to name.

- 2026-07-21 — an `id` read now runs the same filters a ranked read runs, and the
  one filter it must never run stays off. `tool_query` returned
  `entity_detail_by_id` and returned *before* `build_query_options` was ever
  called (`src/mcp/tools_query.rs:133`), so `kind`, `source`, `scheme`,
  `min_conf`, `since`, `before`, `valid_at` and `as_of` were accepted by the
  schema, parsed by nobody, and silently dropped — `query {id, kind: "claim"}`
  answered with a `Fact`. A filter meant one thing on `text` and nothing on
  `id`. The branch now builds `QueryOptions` first and puts the resolved row
  through `retrieval::score::matches_filter` (`src/retrieval/score.rs:226`) —
  the same predicate the ranked read uses, not a second copy — and a malformed
  `since` is an error on the id path instead of a shrug. Resolution stayed
  single: `entity_detail_by_id` and the tool both go through one new
  `resolve_by_id`, so prefix and cold tier still resolve exactly one way.

  **A bare `query {id}` still filters nothing**, and that is load-bearing rather
  than incidental: `QueryOptions::default()` leaves `valid_at`/`as_of` unset, so
  an expired row keeps arriving *served and flagged* (`expired`/`valid_until`),
  which is the retired item 91 `[retrieval]` decision. Reverting the `.filter`
  fails `id_read_drops_a_row_the_kind_filter_excludes` *and* the explicit
  `valid_at` complement inside `bare_id_read_still_serves_an_expired_row_flagged`
  — neither direction is decorative, and the three `graph_ops.rs` retention tests
  needed no edit because they call the resolver, which never filtered.

  **This does not deliver ACL.** It is item 18's id-path bullet and nothing else:
  `matches_filter` still has no ACL predicate and `QueryArgs` has no
  `principals`, so what shipped is the *route* an ACL rule would travel, not the
  rule. What it does buy is that the rule can no longer be walked around —
  before this, an ACL check added to `matches_filter` would have been decorative,
  because `kern get`, routed to this tool by item 9, bypassed the filter
  entirely. Item 18 stays open with three bullets left, still decided alongside
  item 24.

  Decided by: fix-the-root — the early return was the root, not the missing
  `kind` check. Special-casing one filter at the branch would have left the next
  one to be dropped the same silent way.

- 2026-07-21 — the pre-commit hook refused this merge twice, and the second
  refusal caught a real loss. Merging `cycle/2` carried its `ROADMAP.md` edits
  into master while adding no new `CHANGELOG.md` entry: the resolution kept both
  parents' entries, which reads as recording but is not, since both were already
  written elsewhere. Ruling 1 counts entries before against after and does not
  care that the words are new to *this* branch.

  Then the second attempt failed with **161 before, 159 after** — and that was
  not the rule being pedantic, it was arithmetic catching a bug in my
  resolution. Splicing the conflict hunk by hand had silently dropped four
  entries that lived outside the hunk I was looking at. Rebuilt as a true union
  instead: take HEAD's file whole, prepend only the entries `cycle/2` has that
  the merge base does not. 161 + 1 + this one = 163.

  Worth stating because merges are where this rule is easiest to rationalise
  away — the work was recorded, on the branch, by the commit that did it, so the
  merge feels like transport rather than decision. It is not. Resolving a
  conflict chooses which version of the repo's own answers survives, and a
  hand-spliced hunk is exactly how four of them stopped surviving without anyone
  noticing. The counting rule found it; reading the diff had not.

  What landed: item 25 measured rather than implemented, its "top structural
  debt" claim withdrawn, item 26 carrying the flat-20ms PageRank figure, and
  `tests/seed_scale.rs` kept as the `#[ignore]`d instrument behind both.
  `docs-check` green at 675 references, no anchors nominated — the first merge
  today that broke no citations at all.

  Decided by: the oracle — the machinery refused a commit, the refusal was
  right twice over, and the fix is a record plus a corrected merge, not a
  bypass.

- 2026-07-21 — item 25 was measured before being implemented, and the
  measurement said to do item 26 instead. `seed_important`'s O(N) scan is real
  — 34 ms and 55% of retrieve at N=100k with the whole corpus eligible — but it
  is O(N x eligible), so it shrinks exactly when a query filters. Item 26's
  PageRank does not: it is a flat ~20 ms per query at N=100k whether 1% or 100%
  of the corpus survives the filter, because the power iteration walks the whole
  adjacency regardless. On an ordinary filtered query that is 6.7x the scan.

  So item 25's standing claim to be "top structural debt in the repo" is
  withdrawn as unmeasured, and both items now carry the table. **No index was
  written.** Building one would have been a real speedup for the one workload
  where an index cannot help — a corpus where everything is eligible — and no
  help for the common one.

  The instrument is kept as `tests/seed_scale.rs`, `#[ignore]`d because it is
  ~11 minutes in release. It is kept rather than deleted for the reason it
  existed: both items are perf claims, and a perf claim whose instrument was
  thrown away cannot be rechecked when the numbers move. It is also why the
  first attempt at this slice died — run under a plain `cargo test`, in debug,
  a 100k-entity build never finished and the agent was killed as stalled. The
  harness now says so at the top.

  Ranking is deliberately NOT reordered here. Position is rank in this file, so
  moving 26 above 25 is a decision of its own; this entry is the evidence for
  it, not the act.

  Decided by: verify-before-claiming — the item asserted a cliff and a priority,
  and only one of the two survived contact with a measurement.

- 2026-07-21 — item 91 `[ingest]` closed: the second dedup gate stopped lying,
  on both counts. What it was lying about was an **id**. `place_document`
  returned an unconditional `Some(doc_id)` — the content hash of the text the
  caller handed in — on every exit except the first dedup gate. When
  `accept_with_dedup`'s wider `entity_idx ∪ gnn_entity_idx` scan deduped, that
  hash named an entity that had just been discarded whole. Everything
  downstream then believed it: `finalize_doc_identity` infers dedup from
  `surviving_id != content_id`, so a content hash coming back read as a fresh
  commit and the caller was told `Committed` for a document that was merged into
  another one, holding an id that resolves to nothing. `finalize_doc_identity`
  was never wrong and is untouched — it was being fed a lying id, and the fix is
  one line: return `Some(result.entity_id)`.

  **Gating the index is the right half, and the alternative was considered.**
  The second half was `lex.insert` running unconditionally in both
  `place_document` and `place_chunks`, after a branch that may have dropped the
  entity. The obvious-looking alternative — reindex the wording under the
  *survivor's* id rather than drop it — was rejected here because it is a
  different, larger change wearing this item's clothes. What was actually in the
  index was a posting for an id nothing resolves to, and `seed_lexical` does not
  filter by graph presence, so that ghost reached `fuse::rrf`, got rescored to
  `0.0` by `find_entity_ref_in_graph`'s `unwrap_or(0.0)`, and spent a slot of a
  bounded seed list. Deleting it is a strict win with no recall to trade.
  Carrying the merged wording forward is a real and separate gap — it is missing
  on *both* gates and always has been, since `merge_duplicate` parks the
  alternate wording on a `Rephrase` reason that is minted with an empty vector
  and that the tick's enrichment pass skips for want of a `to`. That is filed as
  item 94, ranked first in tier 8, not smuggled in here.

  Proven by reverting each half separately rather than by the tests passing:
  putting `Some(doc_id.to_string())` back fails
  `a_second_gate_dedup_reports_deduped_and_the_surviving_id` on
  `left: 0302188… right: 64989cc…` while leaving the lexical test green;
  ungating the two `lex.insert`s fails
  `place_chunks_second_gate_keeps_the_discarded_id_out_of_the_lexical_index`
  alone. 862 tests pass, and the e2e floors are unmoved at
  0.9306 / 0.9722 / 0.9471 — which is a statement about a 36-fact corpus with no
  near-duplicate pair in it, not a measurement of this fix. Also folded in:
  `place_document` no longer clones the whole `Entity` under the write guard,
  since `tid`/`joined` are the only things read off it afterwards and neither
  depends on guard-held state.
  Decided by: name-the-tradeoff — "gate the insert" and "reindex under the
  survivor" are not the same fix, and closing the item on the first without
  filing the second would have banked a silent recall gap as done work.

- 2026-07-21 — the acquittal marker silenced a real breakage on its first
  contested use, and the merge caught it. `cycle/1` adjudicated
  `FEATURES.md:608` as a false positive and stamped it
  `<!-- docs-check: anchor-ok -->`; master had independently re-pointed the same
  citation to `:625-626`. Reading both targets settles it — `:625-626` is
  "Prompts and resources are served on the standalone path only", the sentence
  actually being cited, and `:608` is the `health` tool's table row. The
  acquittal was wrong, and unlike a wrong re-point it would never have been
  nominated again: a marker is permanent, so it converts a visible breakage into
  an invisible one.

  That is the cost of the escape hatch, and it is worth naming now rather than
  after it hides something load-bearing. **An acquittal is a stronger claim than
  a re-point** — a re-point says "this line is the one", which the next audit
  re-checks; an acquittal says "no audit will ever look here again". The rule
  the marker needs, and does not yet have: acquit only after reading the target
  and finding it correct, never because the nomination is inconvenient. Three
  markers survive this merge and each was verified by reading: `FEATURES.md:200`
  (cold backfill, whose meaning lives in the surrounding lines), `README.md:352`
  (the `move` table row) and `FEATURES.md:54` (which does say "Also carries an
  `acl`").

  Merging `cycle/1` also proved the re-points themselves hold up: 36 nominations
  on master before, 2 after, both genuine and both fixed —
  `crdts-federation.md` cited `src/commands.rs:1016` for the `start_delta_flush`
  wiring, which is `g.network_id.clone()`; the call is at `:1039`.

  Decided by: verify-before-claiming — the conflict was settled by opening both
  candidate lines rather than by preferring either branch, which is the only
  reason the bad acquittal surfaced at all.

- 2026-07-21 — three parallel cycles reconciled into one doc set, and item 93's
  content check earned its keep on the first merge it saw. `cycle/3` (item 19)
  merged master carrying item 93 and item 91 `[retrieval]`; every source file
  auto-merged and only `CHANGELOG.md` and `ROADMAP.md` conflicted, both because
  all three sides had prepended to the same two lists. Nothing was dropped: the
  changelog keeps all seven conflicting entries newest-first by commit time, and
  "Closed and verified" keeps item 19, item 91 `[retrieval]` and item 88 in
  closure order above the items already there. Item 75 took HEAD's rewrite —
  the half of it verified false — over master's, which had only re-anchored the
  paragraph HEAD deleted; master's three corrected
  `diskann-disk-index.md` line numbers were carried across into it, since those
  were right and HEAD's were stale.

  **The number 91 was allocated twice and the collision became a live wrong
  pointer.** Item 18's fourth bullet said "item 91 is the same unfiltered path
  dropping `valid_until`", written about the `[retrieval]` 91 — which has since
  closed and retired its number, leaving the open `[ingest]` 91 ("the second
  dedup gate lies about what it did, twice") as the only thing that spelling
  resolves to. A reader following it landed on an unrelated item. Repointed at
  the retired item by title, and the bullet now also records what the closure
  actually did to it: 91 `[retrieval]` shipped a *flag*, not a filter, so it
  bought this bullet nothing — an ACL denial cannot ride on the row it denies.
  The other reference, in item 88's closure note, was re-read and does mean the
  open `[ingest]` one; it is tagged now rather than left to the number.

  **Twenty-two citations were repointed, and twelve of them `docs-check` found
  by itself** — the first time it has caught anything, because until item 93
  landed it checked only that the cited line existed, and every one of these
  did. Five of the twelve were `ROADMAP.md` -> `FEATURES.md` (`trnsprt`
  pooling, hub↔node version skew, the Ollama retry/backoff gap, `promote`, the
  WSL2 Ollama URL), all shifted by the +16 lines the two `FEATURES.md` edits
  added above them. Seven were `src/` citations moved by the *code* merge, and
  those are the ones no amount of care on either branch could have prevented:
  `with_graph` 442 -> 453 (twice), `wire_fetch` 1015 -> 1038 (twice),
  `validate_fact_source` 118 -> 131, `validate_conf` 116 -> 129,
  `maybe_self_heal_store` 426 -> 424. The other ten came out of reading the same
  pass by hand and were never nominated — `cmd_hub_merge` 648 -> 746 in two
  files, the RPC-auth gap, the watcher's off-by-default paragraph, the `move`
  tool row, the prompts/resources note, the README version pin — which is the
  measured 33% recall the checker was shipped admitting to.

  Master had already resolved item 19's merge independently as `88f1201`, so
  the same two-parent docs conflict got resolved twice, in two trees, by two
  agents who could not see each other. Both are sound; this one is kept because
  only it carries the item 91 repoint above. Taking it wholesale was checked
  rather than assumed — 162 changelog bullets against master's 161, the extra
  being this entry, item 19's entry reordered to the top, nothing dropped.
  Decided by: verify-before-claiming — two resolutions of one conflict is not a
  thing to average, it is a thing to diff and choose between on evidence.

  Counted rather than eyeballed: nominations went 39 (master) -> 51 (merged) ->
  36 (repaired), and the set of nominated targets present in neither parent is
  now empty. The 36 standing are the pre-existing ones, not new damage.

- 2026-07-21 — item 19 closed: deleting a source cascades into the graph.
  `forget_by_source(scheme, object_id, force)` resolves every entity whose
  `Source` matches the pair across all resident kerns and removes it through the
  existing `forget_entity`, reachable as the MCP tool `forget_by_source` and as
  `kern forget --source <scheme>://<object_id> [--force]`.

  **The Fact guard was in two places, and only one of them was obvious.**
  `forget_entity` refuses a local Fact and returns `Err("cannot forget a fact")`
  — that is the guard the item describes and the one a reader finds first. But
  `remove_entity` (`src/base/reason.rs`) carries a *second*, silent one: it
  returns early on an active local Fact with no error and no signal. A `force`
  that lifted only the outer guard would therefore have passed the inner one's
  early return, counted the entity as removed, and reported
  `removed_entities: 1` while the entity was still in the kern — success
  printed over a silent refusal, which is a worse failure than the refusal it
  replaced. Both take `force` now. This was verified by neutering it rather
  than by reading it: making `force` inert inside `remove_entity` fails three
  tests at three levels (`base::reason`, `commands::graph_ops`,
  `mcp::tools_mutate`) plus the e2e, and the graph_ops failure is precisely
  the described defect — the `removed_entities == 2` assertion passes and only
  the "the Fact is actually gone" assertion fails. GC is the call site that
  must never gain this: `src/tick/stigmergy.rs` passes `false`, and no call
  site in the tree passes `true` outside the tests that prove the bypass.

  **The selector is `<scheme>://<object_id>`, and the key is not `source_id`.**
  `source_id()` hashes scheme + object + *section*, so keying the sweep on it
  would forget a single chunk of a document and silently leave the rest —
  exactly the half-deletion the item exists to prevent. The pair
  `(Source::scheme(), Source::object_id())` is what the graph actually stores,
  so that is the key. On the CLI the `://` spelling is the URI shape `Source`'s
  own doc comment uses; everything after the separator is the raw `object_id`,
  **not** a parsed URI path, because re-deriving it from a
  `ticket://<system>/<id>` spelling would guess at which half is which. A
  scheme outside the known set is a caller error; an unknown *object* is a
  legal no-op that removes 0 without erroring, since the host deletes what it
  has and kern reports what it had.

  **A third response field beyond the two specified.** `kept_facts` was judged
  and kept, not accepted. Without it a source composed only of local Facts
  answers `removed_entities: 0`, which is byte-identical to the answer for a
  source that was never ingested — so an *incomplete* deletion reads as a
  complete one and `--force` is undiscoverable. For a host-deletion cascade,
  silently leaving rows behind after "delete this source" is the one outcome
  that must be impossible to mistake for success.

  Also fixed while proving the flag: `#[arg(long, requires = "source")]` does
  **not** fire for a `SetTrue` flag in clap 4.6, so `kern forget --force <id>`
  was parsed, accepted and the flag ignored. Confirmed by re-adding `requires`
  and observing clap let the invocation through anyway; the pairing is enforced
  in `dispatch` instead, and `kern forget --force deadbeef` now prints
  `--force applies to --source <scheme>://<object_id>, not a single thought ID`.
  `FEATURES.md` had been written claiming the clap-level `requires` was what
  enforced it, which was the very thing measured false — corrected.

  **Decided by:** verify-before-claiming, and fix-the-root. Every load-bearing
  claim in the implementation report was checked by breaking it: the second
  guard by making `force` inert, the daemon-routing proof by deleting the
  `data_dir` repoint and confirming the control assertion then fails (without
  the blinding `kern list` does see the entities on the CLI's disk, so the
  blinded run is what makes "the forget landed in the daemon" the only
  reading), and the clap finding by restoring `requires`. Fixing only the outer
  guard would have been the symptom fix; the root is that removal immunity is
  enforced where removal happens.

- 2026-07-21 — the 38 nominations were worked, and the two fixes item 93 named
  landed. Every anchor was opened at the cited line and adjudicated by reading
  it: **27 real drift, 11 false**. All 27 were re-pointed and the new target read
  back before it was written — 29 anchors moved, counting two bare `:NNN`
  continuations the regex cannot see. `bayesian-belief.md:16` was fixed by hand
  as promised: `src/base/types.rs:66-75` is `EntityStatus`, `ReasonKind` begins
  at 77, and the checker still cannot see that breakage because the stray
  "entity" match is still there.

  The tokeniser now keeps three characters instead of four and runs a light
  suffix stripper with consonant-undoubling, so `stemmer` and `stemmers` both
  reach `fn stem` and `acl`, `rrf`, `hub`, `dim` and `gpu` are words again. The
  stopword list grew to match: three-letter articles and pronouns are back in
  play, and Rust's boilerplate (`let`, `pub`, `self`, `new`) is in the list
  because a match on it proves only that the target is Rust. Eight of the eleven
  false positives disappear on their own; three carry the acquittal marker,
  because their targets are a bound check and two single-word table lookups —
  shapes no tokeniser reaches.

  **Measured, before and after, against a hand-adjudicated truth set: precision
  71.1% → 89.7%, false-positive rate 28.9% → 10.3%, and recall 96.4% → 92.9%.**
  Recall *went down*. The three-character floor that acquits `acl` also acquits
  `gpu`, and `gpu` plus `kern` clears the two-word prose bar over an anchor that
  genuinely under-covers what cites it. That is the camelCase trade paid a second
  time and it is written down the same way. A prose bar of 1 measures 96.3%
  precision at unchanged recall and was declined — five prose-to-prose anchors is
  a sample, not evidence.

  Each new rule was proved by breaking it: reverting the floor, disabling
  undoubling, dropping the stemmer, and removing `new` from the stopwords each
  make `--selftest` fail with the assertion written for it, and the fixture
  carries a firing case beside every quiet one. `--strict-anchors` exits 0 on
  this tree, but 10.3% is not near zero and the claim is not made: symbolic
  anchors are still the answer.

  Decided by: verify-before-claiming — every re-point names a line that was read
  back, and the recall regression is reported because it was measured, not
  because it flattered the change.

- 2026-07-21 — `docs-check` now reads the line it is pointed at. Item 93's
  second candidate closure landed: every anchor carrying a line number gets its
  citing block's content words compared against the cited line's, and an anchor
  that shares too few is **nominated**, not failed. Tokens lowercase, split on
  `_` and on the camelCase boundary, keep four characters or more, drop a small
  stopword set. The bar splits by target kind — two shared words for prose,
  total silence for code — because prose citing code shares almost nothing by
  design, and a two-word bar everywhere nominates 117 of 655 anchors, which is
  the same as nominating nothing. Split, it nominates 38.

  The design constraint was the whole point and it is honored: nominations print
  under their own heading, say plainly that they are suspicions, and **do not
  touch the exit code**. `python3 scripts/docs_check.py` exits 0 with all 38
  standing; only genuinely dead references still exit 1. `--strict-anchors` is
  the opt-in that makes them fatal, there for a CI that has decided to trust
  them, which nothing has yet. An adjudicated anchor is silenced in place with a
  trailing `docs-check: anchor-ok` comment — the historical marker's idiom,
  counted only outside backticks so a page can still quote it. Item 93's own
  paragraph is its first user, since its `FEATURES.md:408-409` was never a
  citation, only an example of one going wrong.

  **The measured false-positive rate is 13 of 39, about 33%**, adjudicated one
  at a time against the real tree rather than estimated. That is a real number
  and it is the reason strict is opt-in. The 26 true ones are the predicted
  damage: `bayesian-belief.md` citing `conf_alpha`/`conf_beta` at a `ChunkPart`
  struct, `FEATURES.md` citing `classify_prompt` at a `return Vec::new();`,
  `README.md` citing the cold-tier drop counter at a closing brace — five of six
  anchors in one `crdts-federation.md` status block are wrong. The thirteen
  false ones share one cause: the target's distinguishing word is under four
  characters (`acl`, `rrf`, `run_hub`) or inflected (`fn stem` against
  "stemmer"). Two are this item's own illustrations and are acquitted in place,
  leaving 38 standing at 32%.

  One tuning step is recorded as the near-wash it measured as, rather than as
  the improvement it looked like: splitting the camelCase boundary silenced two
  false positives **and one true one** — `bayesian-belief.md:16` cites
  `src/base/types.rs:66-75` for "the seven kinds that exist" when 66-75 is
  `EntityStatus` and `ReasonKind` starts at 76, and that breakage now hides
  behind a stray "entity" match. Precision 64% → 67%, recall 27 → 26. Kept on
  the principle that prose and code should tokenise alike, not on the numbers.

  The selftest carries the proof rather than the claim: a fixture page cites a
  matching line, a mismatched line, and the mismatched line again with the
  acquittal marker, and asserts exactly one nomination naming the mismatch. Both
  directions are asserted, because a checker never observed firing proves
  nothing — which is how four false-green tests got caught here today.

  Decided by: name-the-tradeoff — the honest 31% is stated in the item, in the
  output and here, instead of being tuned away into a number that would have
  made the checker look ready to gate when it is not.

- 2026-07-21 — retention now reaches the id read surface, and it does it by
  **flagging, not hiding**. Item 91 `[retrieval]` closed.

  The gap was real and every line of the item checked out against source:
  `drop_expired` had one call site, the ranked path, and `tool_query` returns
  `entity_detail_by_id` before any `QueryOptions` exists — so a fact ingested
  with `--retention-secs 60` vanished from `kern query` after the deadline and
  was still served in full by `kern get` and MCP `query{id}`, forever, because
  GC never reads `valid_until` and a non-superseded `Fact` is GC-immune.

  The item prescribed a filter on the id path. It did not ship, and the reason
  is the question the item deferred rather than an easier fix. **An id is a
  direct question and deserves a direct answer.** `kern query` is asked "what is
  true now" and is right to drop an expired claim; `kern get <id>` is asked
  "what is this row", and answering `thought not found` about a row sitting on
  disk is a false statement with no way for the caller to check it — not a
  softer failure than serving it, a harder one, because it is unfalsifiable and
  permanent. So the resolver annotates: `expired` and `valid_until` on the JSON
  whenever a retention is set, an `Expired:` line on the `kern get` printout.
  Same shape as the `cold` flag that already rides on that JSON, for the same
  reason — the caller should not have to infer a fact about a row from its
  absence.

  **Why not-found lost.** It matches the ranked path's semantics, which is a
  real argument and the item's own recommendation. It lost because item 9
  deliberately widened this surface — `query{id}` takes a prefix and falls back
  to the cold tier precisely so `kern get` loses nothing by routing — and a
  silent drop narrows it back to an error message indistinguishable from a
  mistyped id. A caller who wants ranked-path semantics on an id already has
  them: read the flag and discard. A caller who wants the row has no way to
  recover it from a not-found. Asymmetric, so the reversible option wins.

  Bi-temporal is untouched and now pinned where it was not before. `as_of` /
  `valid_at` still skip `drop_expired`, and that escape had unit tests only on
  the predicate — the same shape of hole that let `valid_until` be honoured by a
  function nothing called. `retrieve_drops_an_expired_claim_from_the_default_path`
  now runs the corpus twice and asserts the named-instant query still returns
  the since-expired claim, so deleting the early return fails at the call site.

  Both new tests were reverted and re-run before either was believed: the id
  test fails `left: Null, right: Bool(true)`, the bi-temporal half fails with
  `["live"]`, and the e2e test fails against a real binary printing the expired
  fact with no marker — which is the defect itself, reproduced. `e2e` floors
  unmoved at 0.9306 / 0.9722 / 0.9471.

  Not bought: item 18's fourth bullet still needs its own guard. It wants ACL on
  the id path, and an ACL denial cannot be a flag on the row it denies — the
  text would ship with it. Item 91's claim that closing it would hand 18 that
  bullet was wrong.

  Decided by: name-the-tradeoff — the item had already chosen the filter, and
  the two failure modes were only comparable once "silently reporting not-found
  for a row that exists" was stated as the cost it is.

- 2026-07-21 — merging `cycle/1` broke **fifteen** line anchors at once, and the
  count is the point. Both branches had just re-pointed their anchors and both
  were right in their own tree; combining them shifted `FEATURES.md` again and
  eleven `ROADMAP.md` -> `FEATURES.md` citations plus four `src/` ones landed on
  unrelated content — "no batch query" on the `health` tool row, "clustering is
  vector-only" on kern idle timeout, `get_or_spawn_unnamed_child` on a blank
  line 90 lines short. `docs-check` was green at every moment, before, during
  and after, because all fifteen lines existed the whole time.

  This is the fourth occurrence today and the mechanism is no longer in doubt:
  **a cross-file line anchor cannot survive a merge that appends to the target
  file, and appending is the only thing that ever happens to `FEATURES.md`.**
  Re-pointing them by hand each merge is not a fix, it is a tax that will be
  paid wrong the first time nobody checks. Filed as item 93 — the anchors want
  to be symbolic (a heading or a stable phrase), or `docs_check.py` needs to
  verify content rather than existence. Until one of those lands, every merge
  touching `FEATURES.md` silently rots its citations.

  Found by an overlap audit rather than by reading: comparing the words of each
  citing sentence against the words of the line it points at flags a wrong
  anchor in one pass, where `docs-check` cannot flag any of them. Three of the
  22 the audit scored "weak" were correct — short targets score low — so the
  tool nominates and a human adjudicates; it is not a gate.

  Decided by: verify-before-claiming — the anchors were re-read against the
  merged file rather than trusted from either parent, which is the only step
  that catches this class at all.

- 2026-07-21 — `test_retention`'s intermittent failure is now ROADMAP item 92
  rather than folklore. Two independent runs saw
  `test_a_retention_expires_the_fact_out_of_query_results` fail, always with
  the CPU-heavy `test_recall.py` ahead of it; a third could not reproduce it in
  six consecutive runs on stashed-clean `src/`. Recorded with that split
  evidence intact rather than resolved to "flaky" or "fine", because neither is
  established and the honest state is "real, load-dependent, not reproducible on
  demand". The reason it is worth an item at all is not the failed run — it is
  that an unrecorded intermittent failure is indistinguishable from a
  regression, so the next person to see it red waves through whatever actually
  broke.
  Decided by: verify-before-claiming — the implement stage's flake claim was
  checked rather than accepted, and the check disagreed with it, so both results
  are in the item.

- 2026-07-21 — `kern graviton add`/`remove` and `kern claim-kind add`/`rm` route
  through the daemon; item 9's unblocked half closed.

  These four were the last shipped subcommands calling `with_graph` with no
  routing in front of it. `with_graph` loads, mutates and calls
  `save_graph_unguarded`, holding no writer lock and doing no epoch check, and
  it writes the whole kern map — so run beside a daemon, `kern graviton add`
  did not add a graviton to the live graph, it replaced that graph with a
  minutes-old copy of itself plus one graviton. The daemon's next persist then
  dropped the graviton too, because its own graph had never seen it. Both ends
  of that, from one command.

  No new tool was needed: `graviton` and `claim_kind` already exist on the MCP
  surface with the same semantics the CLI wants. The shape copied is
  `cmd_forget`/`cmd_degrade` — route first, `with_graph` only on `NoDaemon`,
  one printer per outcome so routed and local output cannot drift. The one
  ordering call worth naming: the graviton add routes *before* it embeds. The
  daemon embeds with its own client and owns the vector it stores, so embedding
  first would spend a model call on a vector nobody keeps.

  **The seam that made it testable.** `route()` already delegates to
  `route_to(&Endpoint::kern(), …)`; `cmd_graviton`/`cmd_claim_kind` now delegate
  the same way to `graviton_at`/`claim_kind_at`. Without that, a unit test of the
  routed path would have to reach the process-global `Endpoint::kern()` — cwd
  plus `XDG_RUNTIME_DIR` — which is neither settable in parallel tests nor
  distinguishable from a daemon the developer happens to be running.

  **Both new tests were reverted to prove they fail.** The unit tests fail with
  "the serving daemon's own graph took the write" / "the serving daemon's own
  graph lost the graviton"; the e2e fails with "the CLI wrote the graviton
  locally after all", and with the control removed it fails one assertion later
  with "the daemon's persist dropped the routed graviton" — so both halves carry
  weight, not just the control. The regression guard was checked the same way by
  dropping the local persist: "the local add must reach the local store".

- 2026-07-21 — a full semantic sweep of every line anchor in `ROADMAP.md` and
  `FEATURES.md`, and two claims that did not survive it.

  **Anchors.** Twenty-seven citations pointed at a line that exists and no
  longer says what it was cited for; `docs-check` was green at 634 references
  before and after, as it has been every time. Ten were `ROADMAP.md` ->
  `FEATURES.md` — the predicted rot, and the counts show why: every one of them
  had drifted *downward* by between 14 and 27 lines, the size of what
  `FEATURES.md` grew by underneath them. `:390` for the 512-bounded tick queue
  had landed on a blank line, `:588-589` for "hand-rolled schemas, no batch
  query" on the `forget`/`degrade` tool-table rows, `:547-549` for the GNN gaps
  on a *different* GNN paragraph — near enough to read as right, which is the
  dangerous kind. Re-pointed to `:408-409`, `:607-608`, `:565-566` and the rest.

  The other seventeen were the same failure in `src/` and `docs/kern/`, and they
  are not merge damage — they are ordinary code motion. `src/tick.rs:195` for
  `spawn_child_clusters` had slid one line onto a blank; `src/commands.rs:1003`
  for `wire_fetch` was twelve lines short of the call, `:966-1040` for
  `start_gossip` thirteen; `src/commands/ingest_cmd.rs:49` for the CLI's
  `clamp_confidence(1.0, "user")` — the citation that carries the entire
  "only the CLI reaches `Fact`" argument — was ten lines above it, on a closing
  brace. `docs/kern/` was worse: the convergence gate `G ≥ 0.6`, the
  `HeatStats` health export, the DiskANN WAL note and the lower-confidence-bound
  formula were all cited at lines holding other content.

  **Claim one, retired: "the remaining two unlocked callers."** Item 9 said
  naming `save_graph_unguarded` had made its two remaining unlocked callers
  visible. Walking the call sites finds three. The third is `with_graph`
  (`src/commands.rs:442`), which loads, mutates and writes the whole graph back
  holding no lock. `cmd_forget` and `cmd_degrade` reach it safely because they
  route first and land there only on `NoDaemon` — but `kern graviton add`/
  `remove` and `kern claim-kind add`/`rm` do not route at all. Beside a running
  daemon they overwrite everything the daemon has committed since they loaded:
  a full-graph clobber, from four shipped subcommands, in the item whose entire
  subject is that race. The item's title and its "one half remains" both said
  `ingest`/`link`; both were undercounting, and both are corrected. This half is
  **not** blocked on item 24 — `graviton` and `claim_kind` assert no trust, and
  the tools to route to already exist with matching semantics.

  **Claim two, promoted to item 91: retention is enforced on one read surface of
  two.** Item 18's "guard the id path" bullet was written as an ACL concern, so
  it read as a cost of work not yet done. It is not: `drop_expired` has one call
  site, on the ranked path, and the id path returns `entity_detail_by_id` before
  any option is built. A thought ingested with `--retention-secs 60` disappears
  from `kern query` after the deadline and is still served in full by `kern get`.
  It does not heal — GC never reads `valid_until`, and the CLI mints at
  confidence 1.0, so the entity is a `Fact` and GC-immune. `e2e/test_retention.py`
  closed item 22 green because it only ever asked the ranked path. Ranked beside
  88 and 89 on 88's own reasoning: a correctness gap in a shipped flag, reachable
  only by opting into retention.

  What both findings have in common is what the pinned pattern predicts one
  level up. A citation that still resolves is not a citation that is still true,
  and a *claim* that was true when written is not a claim that is still true
  either. `docs-check` proves neither; only reading the source does.

  Decided by: verify-before-claiming — every anchor here was opened and read
  against what the citing sentence asserts, and the two claims that fell were
  claims nothing in the toolchain was ever going to fail on.

- 2026-07-21 — merging `cycle/2` re-broke four `ROADMAP.md` -> `FEATURES.md`
  anchors that were correct in the branch, and that is a property of the merge
  rather than a mistake by either side. Both branches appended to `FEATURES.md`,
  so every line number below the earlier insertion point moved once the two
  histories combined: cycle/2's `:833-834` for "the LLM client is Ollama-centric,
  no retry/backoff" landed on the gossip `RateLimiter` paragraph, `:1018-1025`
  for "the watcher is off by default" landed on gossip Gaps, `:732` for "no
  `promote`" landed mid-sentence in the daemon-race note, and `:1042-1043` for
  the WSL2 NAT note landed on a `**Where.**` line. Re-pointed to `:860-861`,
  `:1025-1033`, `:759` and `:1068` and each read back before committing.

  `docs-check` was green at 634 references across the whole episode, before and
  after, because all eight lines existed the entire time. This is the third
  time today that check has passed while the citations underneath it were
  wrong, and the pattern is now specific enough to name: **a cross-file line
  anchor cannot survive a merge that appends to the target file, and nothing in
  the repo detects it.** Anchors into `FEATURES.md` are the ones that rot,
  because it is the file that grows.

  The conflicted hunks themselves were kept from `cycle/2`, whose retirement of
  the "`.gitignore` parsing is approximate; no rename tracking" claim is
  verified against source: `IgnoreRules` builds a real `Gitignore` through
  ripgrep's `ignore` crate, and `WatchKind::Renamed {from, to}` carries both
  endpoints. The narrower gap it leaves open — `build_record` discards `from`,
  so a move-plus-edit gets a new id under a new `external_id` and supersede
  never fires — is what now stands in its place.

  Decided by: verify-before-claiming — every re-pointed anchor was read back
  against the merged file rather than trusted from either parent, which is the
  only step that would have caught this.

- 2026-07-21 — item 88 closed: a retention that lands on a duplicate is applied,
  not dropped. Item 22 shipped `retention_secs` the day before and it reached
  `valid_until` only where an entity was *created*; a near-duplicate re-ingest
  reported `deduped` and left an entity that never expires. There were **two**
  dedup gates swallowing it, not one — `find_duplicate` in `place.rs`, and
  `accept_with_dedup`'s own wider `find_duplicate_hit`, whose `dup` branch drops
  the incoming `Entity` whole. Both now carry the deadline through
  `merge_duplicate` into a single writer, `accept::merge_valid_until`.

  **`min` of the two deadlines, not last-writer-wins, and that was the
  decision.** LWW is what `valid_until` already uses on the *merge* path
  (lamport + producer, `base/merge.rs`), so LWW here would have been the
  consistent-looking choice. It was rejected because a TTL is a **ceiling**, not
  an opinion about a value. Under LWW a near-duplicate carrying 30 days that
  happens to arrive after a deliberate 1 hour voids the 1 hour, and federation
  delivers deltas in arbitrary order, so *which* retention survives would depend
  on network timing. `min` is commutative, associative and idempotent — every
  replica converges on the same deadline no matter what order the writes land
  in. `None` is +∞: `min(∞, t) = t` lets an untimed entity accept a deadline,
  and `min(t, ∞) = t` means omitting `retention_secs` is *no opinion*, not
  "make this permanent".

  **Accepted cost, named in the tool schema rather than hidden:** ingest can
  therefore only ever **shorten** a TTL, never lengthen one. There is no
  ingest-shaped way to say "actually, keep this longer" — that needs an explicit
  update path, or `forget` + re-ingest. This is the right trade for a retention
  feature, where the failure that matters is data outliving its deadline, but it
  is a real limitation and callers are told about it in the `retention_secs`
  description.

  **An orphan delta was found and fixed on the way.** `place.rs` stamped the
  lamport/producer and pushed the `ValidUntil` `PendingDelta` *before* calling
  `accept_with_dedup` — so on a gate-2 dedup it gossiped a deadline for an id
  that never entered any kern. The stamp moved *after* accept, onto
  `result.entity_id`, guarded by `!result.deduped`; the deduped case is handled
  inside `merge_duplicate` against the survivor. A delta is now queued only when
  the stored deadline actually moves, or when it was never stamped — which is
  exactly the freshly placed entity carrying its own deadline in.

  **Decided by:** verify-before-claiming, then fix-the-root. Every claim was
  tested by breaking it rather than by reading it. Reverting gate 1 alone fails
  three `place.rs` tests and leaves the gate-2 test **green**; reverting gate 2
  alone fails **only** the gate-2 test — proof the two tests exercise two paths
  and neither would pass either way. Deleting the `push_delta` while keeping the
  stamp initially failed only the two dedup tests, which exposed a real coverage
  hole: nothing asserted the **non-dedup** delta, the one federation depends on
  for an ordinary retention-carrying ingest. That assertion was added
  (`a_configured_retention_stamps_valid_until_on_every_placed_entity` now checks
  object id, lamport and producer against the entity), and the same mutation now
  fails all three. Root-cause over patch: the fix is one writer both gates reach,
  not a second copy of the rule in `update_existing_entity`.

  **The e2e's clock handling is an environment fix, and was confirmed as one
  independently.** `e2e/test_retention.py` was ~50% flaky before this change.
  Measured on this box: `CLOCK_REALTIME` is stepped **backwards ~2.80s every
  ~32s** by WSL2 hv time sync — 60s of monotonic time advanced realtime by only
  54.4s, and one `sleep(7)` in three advanced it by 4.23s. `valid_until` is an
  absolute instant compared against `SystemTime::now()`, but `time.sleep` waits
  on the *monotonic* clock, so a `sleep(RETENTION + 2)` could return with the
  deadline still in the future. The waits now key on realtime and poll until the
  fact stops being delivered, bounded by a monotonic cap. This does not soften
  the test: with `merge_valid_until` neutered,
  `test_a_deduped_ingest_still_applies_its_retention` fails on the cap with the
  fact still delivered.

  Left behind as item 91: the second gate still returns `doc_id` rather than the
  survivor id, so `finalize_doc_identity` reports `committed` on a gate-2 dedup,
  and both placement paths insert the discarded id into the lexical index. Both
  predate this work and are on the plain path with no retention involved.

- 2026-07-21 — item 22 closed: per-source TTL has a writer. The reader half
  (`score::drop_expired`) had been enforcing `valid_until` on every retrieve
  against a field nothing ever set — `new_statement_entity` was called with a
  literal `None` from both the document path and the chunk path, and the LWW
  lamport/producer stamping and pending delta sitting right above those calls
  had never fired once. They fire now.

  **The unit is integer seconds, not a duration string, and that was the
  decision.** `retention: "30d"` reads better in a tool schema and is what a
  host would write in a config file. It was rejected because kern has no
  duration parser and every duration in the tree is already a bare `u64` named
  `*_secs` — `poll_secs`, `done_retention_secs`, `COLD_EVICT_WARN_SECS`. Adding
  a string form for this one field would mean either a parser nothing else
  uses, or two spellings of "how long" in the same config. `retention_secs` it
  is, in both entrances. If a `kern.toml` key later wants `"30d"` (item 89),
  the parser is a strictly additive layer above `valid_until_from_retention`,
  which stays the one conversion from duration to instant.

  **Absolute on the wire, duration at the boundary.** `DirectJob` — the durable
  direct-intake payload, and the *default* async MCP path, since
  `intake.enabled` defaults true — carries the resolved `valid_until`, not the
  seconds. A duration serialized into a file that may sit a whole poll interval
  before it drains would restart on the drain, and the deadline the caller
  asked for would depend on how busy the daemon was. For the same reason the
  CLI resolves the deadline **once, before** the guarded write-retry loop: a
  refused-stale flush reloads and re-runs the whole ingest, and a deadline
  computed inside that loop would slide out by however long the retry took.
  `0` means never, matching every other unset knob; a duration that overflows
  the clock is an error that writes nothing, rather than degrading to no TTL —
  which is the one failure mode a retention feature must not have.

  **Decided by:** verify-before-claiming. Two things the implementation report
  asserted were checked by breaking them rather than by reading them. Reverting
  `place.rs` to its two hardcoded `None`s makes `e2e/test_retention.py` fail on
  the expiry assertion, not on setup — so the e2e proves the writer, not merely
  that the binary runs. Reverting the per-job `Config` overlay in
  `drain_direct_once` makes `drain_direct_once_ingests_and_archives_end_to_end`
  fail on "the retention survives the durable intake round-trip" — so the
  direct-intake leg is load-bearing and tested, not decoration. Also checked
  against the live surface rather than the schema source: `tools/list` over
  `--mcp-stdio` returns `retention_secs` as an `integer` with `required:
  ["text"]`, and `kern ingest --help` lists the flag with its `0 = never`
  wording. e2e floors unmoved: 0.9306 / 0.9722 / 0.9471.

  **The reported flake did not reproduce, and the mechanism given for it is
  impossible.** `base::store::tests::a_cold_tier_pinned_at_capacity_counts_every_eviction_but_logs_once`
  was reported failing once and attributed to a process-global "log once" guard
  raced by a concurrent test. There is no such guard: `cold_evict_warn` is a
  per-`Store` instance field (`src/base/store.rs:276`) holding a 300-second
  `LogThrottle`, the test's subscriber is thread-local, and nextest runs every
  test in its own process — nothing concurrent can reach it. Ten full-suite runs
  and twenty-five isolated runs were all green, and `src/base/` is outside this
  diff entirely, so the code is byte-identical to the base commit. Recorded as
  unreproduced rather than as fixed or as explained.

- 2026-07-21 — `kern intake drain` routes through the daemon. It is the last
  part of item 9 that was never blocked on anything but a missing tool, and the
  defect it closes is a race rather than staleness: `drain_once` reads the intake
  directory and archives each entry it commits, so a CLI drain running beside the
  daemon's own poll loop distilled the same transcript twice — two LLM calls —
  and then raced it for the archive move. `intake_drain`
  (`src/mcp/tools_intake.rs`) is one immediate pass of `ingest::intake::drain_now`
  inside the daemon, returning `archived`; `drain` (`src/commands/intake_cmd.rs`)
  routes first and falls back to the in-process pass, now `drain_locally`, on
  `NoDaemon`.

  **The tradeoff, taken rather than re-decided:** this puts a mutation on the
  unauthenticated RPC socket (item 24). `gc` and `pulse` are already there, and
  `drain` takes no arguments — no caller-supplied content, no trust claim, the
  queue directory comes from the daemon's own config — so unlike `ingest`/`link`
  it does not widen item 24's hole in a new way. Those two stay blocked.

  Only the archived count crosses the socket. The graph is the daemon's, but the
  queue is a directory both processes can already see, so the before/after scan
  and the whole report stay local and both paths print through one shared tail —
  routed and local output cannot drift because they are the same lines.

  Two things the e2e needed and one it exposed. The fake LLM echoed every chat
  prompt back, which the distill prompt can never accept: `parse_claims` spans
  the first `[` to the last `]`, and the prompt's own "output []" puts prose
  inside that span, so an echoed distill prompt always parses as garbage and no
  `.txt` transcript could ever have drained under test. `fake_llm.distilled`
  answers that one prompt in its own contract. And `e2e/conftest.py` hard-coded
  `<repo>/target/debug/kern`, which is a directory that never gets written when
  `build.target-dir` points at a shared cache — it now asks `cargo metadata`.

  Each new test was watched failing against reverted code before being kept:
  routing removed → `the drained claim never reached the daemon: out=no results`;
  dispatch arm removed → `err=unknown tool: intake_drain` (reachable only after
  an `#[allow(dead_code)]`, because `-D warnings` fails the build on the orphaned
  method first — a tighter guard than the test); schema removed →
  `intake_drain must appear in tool_schemas() exactly once`. The `NoDaemon` test
  passes under all three reverts by design: it guards the fallback, not the
  route.

  `cargo nextest run --workspace` 833 passed. `cargo test --doc --workspace` ok.
  `just check` clean. `just docs-check` 628 references, selftest OK. `just e2e`
  21 passed, 4 skipped, floors unmoved: recall@1 0.9306, recall@5 0.9722,
  MRR 0.9471.

  Decided by: name-the-tradeoff — the socket cost is recorded here instead of
  being re-argued at the next mutation, and the reason it is narrower than
  `ingest`/`link` is stated rather than assumed.

- 2026-07-21 — a semantic reconcile of the oracle files, on the premise
  `docs_check.py` states about itself: it proves a cited line **exists**, never
  that it still **says** what it was cited for. Every claim in `FEATURES.md` and
  `ROADMAP.md` asserting a current gap was opened against its source. Five were
  false, and the pattern in them is worth more than any single fix: **a gap block
  outlives the gap.** The code that closed it moved on; the sentence describing
  it did not, and nothing in the toolchain can tell the difference. All five had
  survived several reconciles that read the prose instead of the source.

  Retired as verified-false, in place:

  - **Routing does not do a vector lookup per level.** `route_to_child_id`
    (`src/base/accept.rs:790`) is a linear scan over the parent's loaded named
    children against each child's stored `graviton_vec` — no index is consulted.
    The cost is O(depth · children), not O(depth · log n), and the "cached
    per-kern centroid" the gap proposed as its *fix* is what `graviton_vec`
    already is. The item recommended the thing the code already does.
  - **Unnamed children are not unbounded per parent.** The routing path goes
    through `get_or_spawn_unnamed_child` (`src/base/accept.rs:552`), one reusable
    holding pen, guarded by three tests. Only tick clustering makes more, one per
    spawnable cluster and on purpose (`src/tick.rs:195`).
  - **The watcher's `.gitignore` parsing is not approximate** — `IgnoreRules`
    builds a real `Gitignore` from ripgrep's `ignore` crate
    (`src/watcher/src/ignore_rules.rs:3`) — **and renames are tracked**,
    `WatchKind::Renamed {from, to}` (`src/watcher/src/event.rs:9`). What is
    actually missing is narrower and now says so: a rename is not re-keyed in the
    graph, so `build_record` (`src/watcher/src/pipeline.rs:48`) ingests `to`,
    discards `from`, and leaves a duplicate Document behind.
  - **Gossip has a per-peer rate limit.** `RateLimiter` (`src/gossip/rate.rs`,
    30/min) runs on every inbound `Question` (`src/gossip/handler.rs:318`).
    `FEATURES.md` said there was none while `ROADMAP.md` items 34 and 37 both
    described it in detail — the two files contradicted each other in the tree,
    which `ORACLE.md` calls an unmade decision, not a typo. Settled in both: the
    true claim is narrower, the `Delta` path (the one that takes the write lock)
    has no budget.
  - **A failing GNN propagation does not re-enqueue every tick.**
    `GnnPropagate` is enqueued only when `do_cluster` did structural work
    (`src/tick.rs:168`), so a quiescent kern retries nothing.

  Renamed because the symbol meant something else: **`KERN_CAP_DISABLED` is not
  a per-kern entity cap.** Its own comment calls it a kern-eviction sentinel. It
  disarms `max_loaded_kerns` (`enforce_kern_cap`, `src/base/graph.rs:216`) and
  `disk_threshold` (the DiskANN spill trigger, `:296`). Item 83 is retitled to
  what is true — nothing bounds memory deterministically — and a per-kern
  *entity* cap for local kerns does not exist at all.

  **The anchor re-point that ran two commits ago moved five citations onto lines
  that exist and say something else**, which is the exact failure this pass is
  for and the reason existence-checking cannot be the last word: `FEATURES.md`
  `:692` for "`unnamed` has no `promote`" landed on the daemon boot list, `:981`
  for the WSL2 note landed on a `**Where.**` line, `:832-833` for connection
  pooling landed on a module list, and the version-skew and watcher anchors had
  swapped places. Eighteen `ROADMAP.md` → `FEATURES.md` anchors are re-pointed at
  the text they cite. So are the stale intra-file ones: `src/base/hnsw.rs`
  (944 → 1042 LoC, four functions ~26 lines off), `src/llm.rs` (861 → 585 LoC,
  five symbols moved) and three `src/gossip/handler.rs` starters.

  `SPECIALISTS.md`'s `surface` brief said `forget` and `degrade` were the only
  commands that route — true until the commit directly below this entry, which
  added `get` and `query`. A specialist brief is read as authority before the
  code is, so it is corrected in the same pass that found it stale.

  One state label corrected, which is the finding that ranks the rest: **§19's
  file watcher is marked `active` and is off by default.** `WatcherConfig::enabled`
  is a `bool` behind `#[derive(Default)]` (`src/config/watcher.rs`), so nothing
  in that section runs unless a `kern.toml` turns it on. `Federation` says "off
  by default" in its own heading; the watcher did not, and a gap in an opt-in
  subsystem does not rank with a gap on the default path. That is why the rename
  finding above sits in item 84 rather than tier 1 — and the finding is now
  stated precisely: it duplicates only on a move *plus* an edit, because ids are
  `content_hash(text)` and an untouched move re-resolves to the same id, while
  `external_id` is the path, so a move-plus-edit gets a new id under a new
  external id and supersede never fires.

  No code changed. `just docs-check` 626 references, selftest OK.

  Decided by: verify-before-claiming — every gap claim was read against the
  source that closed it rather than against the document that repeated it, and
  the five false ones were all reachable from a citation that a passing
  `docs-check` had already blessed.

- 2026-07-21 — the read-side routing was built twice, by two sessions that could
  not see each other, and the merge is a reconciliation rather than a pick. Both
  branches routed `kern get` through the `query` tool with the local load as the
  `NoDaemon` fallback; both moved `find_entity_by_prefix` into `base::search` and
  added `EntityKind::from_u8` / `ReasonKind::from_i32` for the discriminants the
  MCP payload carries. What each side had alone is what survived.

  From `cycle/1`: `kern query` routes too — which the other side had ruled out
  because the `query` tool returned no path chains, a gap closed by making the
  tool return them, so the CLI's "--- Connections ---" section survives the trip.
  With it, `retrieval::score::delivery_cap` as one owner for the delivery cut, so
  a routed query cannot answer with fewer hits than the local one; a single
  `entity_detail_by_id` behind both the tool and `kern get`, so prefix and cold
  lookups cannot diverge between them; and `e2e/test_daemon_reads.py`, where
  `search`/`list` going blind against an emptied data dir is the control that
  proves `get`/`query` answered over the socket.

  From master: `kern link` flushing through `save_graph_guarded` with
  `save_graph` renamed to `save_graph_unguarded` (a separate half of item 9,
  kept whole); the prefix test built on a full-length id rather than a
  one-character one that is an exact match dressed as a prefix; the
  daemon's-unflushed-state test, a case the live-graph test does not cover; and
  a `"cold": true` flag on the detail JSON so a reader need not match on the
  `(cold)` sentinel the printer shows.

  Where the two conflicted: the shared printer prints `{:?}` kind labels
  (`Question`), not `as_str` ones (`question`) — the wording `kern get` has
  always had, since a printer extracted to prevent drift should not itself be
  the drift. One printer, one resolver, one delivery cap.

- 2026-07-21 — item 9's **read** half is closed: `kern get` and `kern query`
  route to a serving daemon before they touch disk, both over the `query` tool
  that already existed (`{id}` for the detail read, `{text, mode, k}` for the
  ranked one), with the local load as the `NoDaemon` fallback. Both paths render
  through one printer over the tool's own JSON, and one id resolver
  (`entity_detail_by_id`) serves the tool and the CLI, so a routed and a local
  `get` cannot disagree about what a prefix resolves to.

  **The bug that only showed up when the two paths were measured against each
  other.** Routing `query` without naming `k` inherits the tool's own default,
  `seed_k` — well under the delivery pool the local read prints. On the 36-fact
  e2e corpus that was 25 hits with a daemon up and 36 without, from the same
  command against the same store: the answer size silently depending on whether
  something was serving. The cap was a five-line expression inlined in
  `filter_delivery`, reachable by nobody else, so the CLI had nothing to ask.
  It is now `retrieval::score::delivery_cap`, one owner, read by the cut and by
  the router both — and `e2e/test_daemon_reads.py` fails on a count mismatch
  rather than on a wrong number nobody would have looked at.

  **Two tradeoffs, named rather than found later.** A routed `kern query` embeds
  with the *daemon's* configured model, so `--embed-model` on that invocation is
  ignored — correct, because the daemon owns the index the query has to hit, and
  the same "the daemon owns it" rule the write half took. And `search` and
  `list` stay local **by decision**: `search` is the raw-ANN probe with no
  matching tool, `list` prints the on-disk kern tree, and routing them would
  remove the only way to see what is actually on disk while a daemon runs. That
  is also what makes them the control in the new e2e — with the CLI's config
  repointed at an empty data dir, they must go blind in the same breath that
  `get` and `query` still answer, which is the only thing that proves the answer
  came over the socket and not off a disk the test forgot to empty.

  Item 9 does not close. `ingest`/`link` (blocked on item 24) and `intake drain`
  (no matching tool) still write locally; the title now names only those.

  **Decided by:** verify-before-claiming — the count divergence was not visible
  in any test, in either code path read on its own, or in the implementer's
  report; it appeared only when the same probe was run through both paths and
  the outputs compared. And fix-the-root: the fix is one owner for the delivery
  cap, not a matching constant at the call site.

- 2026-07-21 — `docs/oracle/` reconciled against the tree, and the interesting
  finds were not in the two files that get audited. `FEATURES.md` and
  `ROADMAP.md` drifted only in arithmetic and anchors — 156 tracked `.rs` files
  and ~42.4k lines, not 155/~42.0k; `src/mcp/*` is 2346 LoC and `src/rpc/*` is
  201; item 82's tick citation and item 84's four `FEATURES.md` line refs had
  all slid under the commits that landed `route.rs` and `claim_standalone`.
  `VISION.md` and `SPECIALISTS.md` are where the live lies were, because nothing
  points a checker at them: `SPECIALISTS.md` still taught "the nine MCP tools"
  (twelve) and "tarpc `KernRpc`" (there is no tarpc in `Cargo.toml`, `Cargo.lock`
  or `src/` — the service is this repo's own `service!` macro), and both files
  still gated the claim standard on ROADMAP item 1 *being open* when item 1 is
  closed and its own closure record says the standard is unchanged by it.
  Restated as it actually stands: the scorer exists, runs against a
  bag-of-words embedder, catches regressions and certifies nothing.

  Two more, same shape. **Repo law 3 named a symbol that does not exist** —
  `tools::dispatch` is in `VISION.md`, `SPECIALISTS.md` and the law itself, and
  in no `.rs` file; the single core every surface actually reaches is
  `mcp::Server::call_tool`, so the law now names it and can be checked. And
  `FEATURES.md` §23 said item 9 was "now reduced to read-side staleness" in the
  same bullet that listed `ingest`/`link` and `intake drain` as still writing
  locally — the item's own title names three things, so §23 now does too. The
  same tarpc line is on `README.md:116`, outside this directory, so it went to
  item 85 rather than being fixed silently. And the preamble announced "two
  headings destroyed" and then explained one of the two as a tier retirement,
  which is one heading; it now says one.

  **Decided by:** verify-before-claiming. Every number and every anchor here was
  read off the tree, never off a neighbouring document; `just docs-check` proves
  existence only, which is exactly why the two files it cannot judge were the
  ones carrying false statements.

- 2026-07-21 — `kern get` routes to the serving daemon, and the reason the other
  three reads did not follow is the finding, not an excuse. Item 9's read half
  says `get`, `list`, `query` and `search` load from disk and can report older
  than live state. Routing looked like four copies of the `forget` change. It is
  not: **the daemon's tool surface is narrower than the CLI's read commands**, so
  a naive route trades staleness for lost capability.

  - `get` — `query{id}` did exact-match only, while `cmd_get` accepts a prefix
    and falls back to the cold tier. Routing it as-was would have turned a hit
    into "thought not found". So `find_entity_by_prefix` moved to
    `src/base/search.rs` and the tool's id path gained both the prefix and the
    cold fallback *first*; only then did routing become a transport swap instead
    of a behaviour cut. Both paths now render through one `print_entity_detail`
    reading the same JSON the tool returns, so routed and local output cannot
    drift — the same discipline the `forget`/`degrade` printers already had.
  - `query` — the tool returns `{entities}` and no path chains, so routing it
    would silently drop the CLI's "--- Connections ---" section. The tool has to
    return chains before this can move.
  - `list`, `search` — no tool exists at all. Adding one is a decision about the
    unauthenticated RPC surface (item 24), not a mechanical port.

  `EntityKind::from_u8` and `ReasonKind::from_i32` exist now because the shared
  printer decodes the discriminants the MCP payload carries; the payload format
  is unchanged, so no agent contract moved.

  **The prefix test passed before the code did, and that is the second false
  green in two changes.** It asked for id `"a"` against an entity named `"a"` —
  an exact match dressed as a prefix, green with prefix matching removed. It now
  uses a full-length id and asks for four characters of it, and fails without the
  widening on "a prefix must resolve through the daemon".

  Evidence: `just check` clean, 829/829 nextest, e2e recall 0.9306 / 0.9722 /
  0.9471 unchanged, docs-check 588.

  Decided by: verify-before-claiming — every routed read was checked against what
  the local command actually resolved, which is what exposed the prefix and
  cold-tier gaps — and name-the-tradeoff for stopping at one of four reads with
  the blocker for each of the other three written down.

- 2026-07-21 — `kern link` stops clobbering a concurrent writer, closing the one
  half of item 9 that needed no auth. `cmd_link` called the unguarded save, which
  writes the whole kern map with no epoch check, so a daemon commit landing
  between the command's load and its flush vanished. It now flushes through
  `save_graph_guarded`, the same refuse-absorb-retry path `cmd_ingest` and
  `intake drain` already used.

  The root, and why the rename is part of the fix: nothing at the call site said
  the plain `save_graph` was conditional. It is safe only under the writer lock,
  which `gc`, `compact` and `reembed` hold and `cmd_link` never did — so the
  hazard was invisible to anyone adding a fourth caller. It is now
  `save_graph_unguarded`, with the precondition on the function. Two remaining
  callers are named by that rename and NOT fixed here, because neither is the
  claimed item and both want their own decision: `cmd_hub_merge`
  (`src/commands/admin.rs`) writes a destination graph it does not lock, and
  `maybe_self_heal_store` (`src/commands.rs`) rewrites during boot recovery.

  **The first regression test for this was worthless and the second one was
  too.** Version one called `save_graph_guarded` itself and asserted the helper's
  behaviour — it passed with the fix reverted. Version two drove `cmd_link` end
  to end, and also passed reverted, for a subtler reason: `cmd_link` loads its
  graph *inside* itself, so an external commit staged beforehand is simply loaded
  and written back, and the race never occurs. Only after extracting
  `link_and_persist`, which takes the already-loaded graph by value, could a
  genuinely stale graph be handed to the flush. That version fails without the
  guard on `the concurrent writer's kern survived the link's flush` and passes
  with it. Both false greens were caught by reverting the fix and re-running —
  the test that is never seen to fail is not known to test anything.

  Evidence: `just check` clean, 827/827 nextest, e2e recall 0.9306 / 0.9722 /
  0.9471 unchanged against baseline, docs-check 586.

  Decided by: verify-before-claiming — the fix was three lines and the honest
  test was the whole job — and fix-the-root, for renaming the unguarded path
  rather than fixing the one caller that happened to be reported.

- 2026-07-21 — verification pass over the standalone-lock commit (`c375c5e`).
  The code and the tests hold: `just check` clean, 826/826 nextest with the
  three new `standalone_tests` among them, doctests clean, 16 passed / 4 skipped
  e2e with recall@1 0.9306, recall@5 0.9722, MRR 0.9471 — bit-identical to the
  0.9306 / 0.9722 / 0.9471 item 86 recorded as the current master baseline, and
  above every floor (0.9000 / 0.9500 / 0.9200), 0 unretrieved.
  `just docs-check` green on 579 references.

  Two doc claims did not hold and were corrected. **`FEATURES.md` §23 item 5**
  closed with "Open as `ROADMAP.md` item 9, now reduced to read-side staleness"
  two sentences after listing `ingest`/`link` and `intake drain` as also open —
  the paragraph contradicted itself, and the shorter claim is the one a reader
  keeps. It now names all three. **`ROADMAP.md` item 9** said the three
  remaining direct writers are "one-shot, guarded by `save_graph_guarded`, so
  they lose a write rather than a whole graph". Two of the three are:
  `cmd_ingest` and `intake drain`'s `flush` both retry through
  `persist::flush_guarded`. `cmd_link` does not — it calls the unguarded
  `save_graph` (`src/commands/graph_ops.rs:195` -> `persist::save_all` ->
  `Store::save_all_kerns`), which writes the whole kern map with no epoch check.
  So a `kern link` racing the daemon still clobbers, and the item now says so
  and flags it as the one piece of item 9 that needs neither auth nor a new
  tool. The tradeoff paragraph above ("`save_graph_guarded` bounds a one-shot to
  losing a write") is left standing as written — it is true of the guard, and
  rewriting a landed entry hides that the exception was found later, not known
  at the time.

  **Decided by:** verify-before-claiming. The claim checked was the prose
  against the call graph rather than against the neighbouring prose, which is
  the only way "guarded" gets caught: the two commands anyone would spot-check
  are guarded, and the third is the one that reads like it must be.

- 2026-07-21 — item 9's last **long-lived** second writer is closed: `kern mcp`'s
  standalone fallback claims the writer lock before it reads the graph, and does
  not boot beside a holder. The route landed earlier today could not reach this
  one, and the reason is the interesting part — a standalone server has no
  daemon to hand the write to, and a *sibling* standalone binds no socket, so it
  is invisible to `Endpoint::kern()`, to `kern status`'s daemon probe, and to the
  hub. The lock is the only thing in the process that can see it. So
  `claim_standalone` (`src/commands/mcp_cmd.rs`) answers `Own` / `Attach` /
  `Refuse`: claim the dir as `mcp-standalone` before `load_graph`, or — the
  holder is usually the daemon this process just spawned, late to bind — spend
  one more attach window and proxy to it, or exit 1 naming the holder.

  **The tradeoff, taken on purpose.** A `kern mcp` that loses this race now
  gives its client no kern, where before it gave one that served fine and
  silently overwrote whatever the other writer grew. That is a real availability
  loss and it is the right trade: `save_graph_guarded` bounds a one-shot to
  losing a write, but a standalone holds a whole graph for hours and flushes it
  wholesale, so the loser's *entire* graph lands last. The attach window is what
  keeps the cost bounded — the common holder answers, and only a genuine second
  writer gets the refusal.

  Item 9 narrowed rather than closed. What is left is `ingest`/`link` (blocked on
  item 24), `intake drain` (no tool exists), and stale reads in
  `get`/`list`/`query`/`search`. The title was re-pointed at those three.

  **Decided by:** fix-the-root. The reachable-looking fix was to widen the
  attach window in `cmd_mcp` so the standalone path fires less often; that makes
  the corruption rarer and leaves it possible. The root is that the standalone
  server never asked whether anyone else owned the dir, and it is the one writer
  no probe can answer that for.

- 2026-07-21 — item 55 was measuring the wrong retention half-life, and eight
  `ROADMAP.md` citations had drifted off the lines they name. The item said the
  two freshness signals are 24 hours for ranking and **7 days** for retention,
  citing `src/base/heat.rs:18`. That line does hold `7 * 24 * 60 * 60` and
  `docs-check` is happy with it — but it is the `HeatConfig::default()` value and
  it is never what runs. `Config::load` applies the preset unconditionally
  (`src/config/mod.rs:104`, `:132`) and `Preset::apply` is the sole writer of
  `heat.half_life_secs`; the default preset is `relaxed`, which sets **30 days**
  (`src/config/preset.rs`). `FEATURES.md:979` already recorded 30d, so the two
  oracle files disagreed and the plan held the stale half. The real gap against
  the 1–2 days `docs/kern/stigmergy-self-improving.md:160-170` derives is 15–30×,
  not 3.5×, and retuning it is now a commit against `preset.rs` rather than a
  config edit — so item 55 is swept together with item 87, and says so.

  The eight citations are the ordinary cost of `FEATURES.md` being edited under
  a plan that indexes it by line: `FEATURES.md` gained ~14 lines above the
  transport, LLM, watcher, config and CLI sections, so items 46, 47, 84 (four
  bullets) and 85 all pointed a dozen-odd lines short — at real prose, which is
  why nothing caught it. Re-pointed to `:832-833`, `:938-939`, `:778-779`,
  `:956-957`, `:683`, `:972-974`, `:567-568`. Three source citations went the
  same way: `wire_fetch` is at `src/commands.rs:1003` (`:1002` is
  `start_entity_sync`), cited twice — item 36 and the closed list; `QueryArgs` is
  `src/mcp/tools_query.rs:78-111`, not a mid-struct slice ending past its own
  brace; and item 25's `seed_important` range excluded the `g.all()` half of the
  product it describes, now `:127-174`.

  Everything else in the sweep held. Items 18, 19, 20, 21, 22, 24, 26, 27, 28,
  29, 30, 31, 51, 56, 57, 62, 64, 69, 70, 79, 81, 82, 83, 84 and 87 were each
  re-read against source and are still true, including the ones easiest to close
  by accident: `principals`/`scope` appear nowhere in the MCP schemas,
  `forget_by_source` / `source_trust` / `ReviewState` / `gini` / `rust-stemmers`
  / speculative decode exist nowhere in the tree, `serve.mcp_addr` still has no
  reader (`src/commands.rs:803` resolves `cli.mcp_addr` alone), and only
  `GnnPropagate` calls `record_task_failure`. Item 9 was verified line by line —
  `route()` has exactly the two call sites it claims, the lock guards exactly
  `reembed`/`compact`/`gc`, and the `ingest`/`link` trust asymmetry it is blocked
  on is real (`cmd_ingest` mints at `clamp_confidence(1.0, "user")`, `tool_link`
  writes `MAX_AI_CONFIDENCE`). No item appears in both the open list and "Closed
  and verified".

  **Decided by:** verify-before-claiming. `docs-check` proves a cited line
  exists; it cannot prove the line still says what it was cited for, and both
  failure modes here — a struct default the loader overwrites, and a citation
  that slid onto neighbouring prose — pass it cleanly.

- 2026-07-21 — item 9's headline re-scoped to match its own body. The title read
  "the route exists; `ingest` and `link` cannot take it yet", which names one of
  the four things still open. The body lists four: `ingest`/`link` (blocked on
  item 24), `intake drain` (no matching tool exists), `kern mcp`'s long-lived
  standalone writer, and read-side staleness in `get`/`list`/`query`/`search`.
  Roadmap titles are the index of what is left, so a title naming a strict
  subset makes the item scan as nearer done than it is — the verified remainder
  belongs in the line people read first. Now: "`forget`/`degrade` route to the
  daemon, the rest do not". No scope moved; the body already said this and the
  code already matched it. Verified against the landed diff: `route()` has
  exactly two call sites, `cmd_forget` and `cmd_degrade`
  (`src/commands/graph_ops.rs:120`, `:261`).

  **Decided by:** verify-before-claiming. The claim checked was the title
  against the body and the body against the source, and only the title failed.

- 2026-07-21 — item 9's open question is answered: **a one-shot CLI write goes
  to the serving daemon, it does not learn to refuse.** The item named two
  candidates and said the first was right without deciding it. Deciding it: the
  refusal branch is dead on arrival, because a daemon is running essentially
  always, and a CLI that refuses whenever a daemon runs is a CLI nobody can
  use. `src/commands/route.rs` is the route — one probe of `Endpoint::kern()`,
  no spawn, three answers (`Done` / `Refused` / `NoDaemon`). A one-shot must
  never conjure the daemon it was looking for, so the probe does not retry and
  does not start anything; an absent socket is the ordinary case, not a fault.
  A daemon that answers the connect **owns the graph**, so a tool error comes
  back as `Refused` and is printed — never retried against the store behind the
  daemon's back, which is precisely the split being closed. `kern forget` and
  `kern degrade` take the route; the old local path is now the `NoDaemon`
  branch, and both paths print through one printer so they cannot drift in
  wording. `tool_degrade` grew `removed_edges` alongside `decayed_edges`: the
  CLI has always printed a reap count, and a routed degrade that could not read
  one back would have quietly printed 0 for every reap.

  **What the decision cannot cover, and why that is a finding rather than a
  shortfall.** `ingest` and `link` cannot ride this route at all, and the reason
  only shows up once you try. The RPC's sole mutation surface is `call_tool` —
  the *agent* boundary. `tool_ingest` clamps to `AGENT_SOURCE` "regardless of
  what `p.source` claims" and `tool_link` writes `MAX_AI_CONFIDENCE`, while
  `cmd_ingest` mints at `clamp_confidence(1.0, "user")` and `cmd_link` at
  `1.0`. Route them unchanged and every Fact a human typed at their own
  terminal silently becomes an agent Claim. Route them with their trust intact
  and you have put a privilege field on a socket with no auth — item 24's hole
  turned into an escalation path. So that half is blocked on item 24, and
  saying so is worth more than shipping the demotion quietly. `intake drain`
  has no matching tool at all.

  **Tradeoff.** The block points *down* the file — item 24 sits in tier 3, item
  9 in tier 1 — and the list was not reordered, which is a real cost: the
  sequencing edge is now stated in prose instead of being visible in the
  ordering. The alternative is worse. The edge binds only the `ingest`/`link`
  half; item 9's other open halves (`kern mcp`'s long-lived standalone writer,
  read-side staleness in `get`/`list`/`query`/`search`) need no auth and hold
  tier 1 on their own, and item 24's severity is an unauthenticated socket, not
  the one caller that wants to trust it.

  **Decided by:** name-the-tradeoff. Both candidate closures were plausible and
  the item had gone a full cycle without choosing between them; the choice only
  became obvious once the cost of each was written down next to the other, and
  the same accounting is what exposed that "route everything" silently pays for
  itself in demoted trust.

- 2026-07-21 — the same sweep run against `FEATURES.md`, which the previous pass
  did not walk. The last audit re-pointed the roadmap's anchors into
  `FEATURES.md`; it never checked `FEATURES.md`'s own anchors into `src/`. They
  had drifted hardest of anywhere in the repo, because `efd34aa` deleted the
  store's compatibility path and every `src/base/store.rs` citation past that
  point slid by 30-60 lines while still resolving: `Store::open` cited `:351`
  landed on a blank line (really `:283`), `cold_search` cited `:685` landed on a
  closing brace (`:629`), `cold_cap` `:712` (`:678`), `compact_dir` `:790`
  (`:756`), `cold_evicted` `:752` (`:718`), `check_embed_stamp` `:473` (`:417`),
  `flush_guarded` `:594` (`:538`). Thirty-six anchors corrected in `FEATURES.md`
  and fifteen in `ROADMAP.md`, including the whole "Closed and
  verified" block, whose proof-of-closure citations were the ones least likely to
  be re-read and so the ones that had rotted furthest — `start_gossip` cited
  `src/commands.rs:900-930` is at `:966-1040`, `do_resolve` cited `src/tick.rs:64`
  lives in `src/tick/tasks.rs:383`, `MAX_AI_CONFIDENCE` cited `:62` is `:69`. A
  closure whose evidence points at the wrong line is a closure nobody can check.

  Two claims were false, not merely mis-pointed. **Item 37** opened with "no
  per-peer rate limit anywhere"; `53af8ac` shipped one — a per-origin `Question`
  budget (`src/gossip/rate.rs`, 30/min, checked at
  `src/gossip/handler.rs:318`) — and item 34 already said so ten items above.
  The item is narrowed to the true statement: the `Delta` path, which is the one
  that takes the write lock, still has no budget. Its four full-corpus loops were
  cited at `:378, 394, 407, 428`, none of which is a loop; they are `:432`,
  `:448`, `:461` and `:482`, and two of the four go through `remote_kern_ids`
  rather than `g.all_ids()` directly — the shape the item describes is right,
  the evidence was pointing at a `pulse` call. **Item 27**'s heading still said
  "three separate places" over a body that enumerates four costs with two closed.

  The generalisation, recorded because it will recur: a docs anchor into a file
  that shrank is more dangerous than one into a file that grew, since deletion
  moves every following line at once and `docs_check.py` stays green throughout.
  The check that catches it is not "does the line exist" but "does the line
  contain the identifier the sentence names", and that is cheap to run.

  Decided by: verify-before-claiming

- 2026-07-21 — every citation in `ROADMAP.md` re-pointed at what it was cited
  for. `docs_check.py` proves a cited line **exists**; it cannot prove the line
  still **says** the thing. That gap had gone systemic: of the 23 distinct
  `FEATURES.md` line anchors the roadmap cites, 22 landed on unrelated prose —
  item 24's "RPC socket has no auth" pointed at "Trained per-kern on the tick",
  item 67's int4 gap pointed at a blank line, item 85 cited `:166` for a
  recall/NDCG claim that had been withdrawn out of the file. `FEATURES.md` grew
  and every anchor into it slid; nothing could see it, because all 563
  references existed. Source anchors had drifted the same way — item 28's
  `src/tick.rs:66` pointed into the panic guard `3a3afa1` added rather than at
  the `GnnPropagate` arm, now `:97`.

  Four claims were not merely mis-pointed, they were false, and each is retired
  in place with the date and the reason. **Item 52** said chunk + mean-pool was
  the unbuilt upgrade path for graviton seeds; `08c9971` shipped it —
  `seed_examples` splits on newlines and `mean_pool` averages, and the source
  comment the item cited as its evidence was deleted in the same commit. The
  item is narrowed to what `seed_examples` deliberately does not split: a
  single-line seed, which still embeds whole. **Item 79** said
  `validate_fact_source` is "called twice"; `216730d` took one site with the
  ingest `kind` arg, so it is called once — the dead-code conclusion gets
  stronger, not weaker. **Item 85** lost five of its nine documentation debts:
  the `move` tool reached both tables, `Entity.acl` reached the field list, the
  four stale `docs/kern/` research claims were corrected at the source, the
  forbidden quality claims were withdrawn in place, and `README.md`/`VISION.md`
  stopped promising a baseline that does not exist. **Item 47** claimed
  `src/config/serve.rs` is "`mcp_token` handling only" while item 84, in the
  same file, cited `serve.mcp_addr` as a reader-less field living there — the
  two contradicted each other and the file settles it.

  Item 30's queue-depth clause is narrowed rather than retired: closing item 8
  gave `kern intake` a `pending=/stuck=/failed=/done=` readout, so the
  file-backed queue reports depth; the in-process `Worker` channel still does
  not, and the distill leg still has no timeout budget.

  The tradeoff in re-pointing rather than removing line numbers: they will drift
  again on the next edit to either file, and `docs_check.py` will stay silent
  about it, because the check that would catch it — does this line still say
  this? — is a reading, not a script. Anchors are worth keeping anyway: a wrong
  line number costs one grep, while no citation at all costs the whole audit.
  What follows from that is a cadence, not a tool: this pass has to be re-walked
  whenever `FEATURES.md` changes length, and the roadmap should prefer symbol
  names over line numbers wherever a symbol is unique.

  `FEATURES.md`'s own corrections landed alongside this pass in `4ba74a7`, which
  swept both working trees into one commit. Recorded here because that commit's
  message describes only the `FEATURES.md` half, and each was re-verified
  against source rather than taken on trust: HNSW delete is the scrub-and-recycle
  it is rather than a tombstone (`src/base/hnsw.rs:136`, `scrub_pending` `:153`);
  the federation gap block drops the closed local-row reach for the two that
  remain, per-peer rate limiting and a divergence signal (`remote_kern_ids`,
  `id_matches_body`, and the reject-and-clamp in `handle_pulse` all exist at
  `src/gossip/handler.rs:544`, `:527`, `:371-377`); the e2e block records that no
  `xfail` remains (none in `e2e/`); `release.yml` joins the workflow list (it is
  in `.github/workflows/`); and the LoC stamp moves to 155 tracked `.rs` files,
  which `git ls-files '*.rs'` confirms.

  One stale name found in passing and deliberately not fixed here, because it is
  source and not an oracle doc: the test at `src/gossip/handler.rs:993` is still
  called `handle_pulse_falls_back_to_root_for_an_unknown_kern` after the fallback
  it names was removed as the security fix, and its body now asserts nothing.

  docs-check green at 572 references — the same existence proof as before, which
  is exactly why it was green at 563 while 22 of 23 anchors pointed at the wrong
  paragraph.

  Decided by: verify-before-claiming

- 2026-07-21 — `FEATURES.md` §23 and the `ROADMAP.md` preamble reconciled
  against the tree, not against each other. Three of the ten ranked improvement
  opportunities described a repo that no longer exists: "a reason edge changes
  no ranking" (closed with item 86's traversal credit), "add `kern status` +
  advisory locking" (shipped — narrowed to the serving half that item 9 still
  owns), and "`HnswIndex::delete` is O(nodes × edges)" (one scrub pass per
  sweep since `pending_scrub`). Each retired in place with the date and the
  reason, the convention entry 9 already used, so the ranking's shape survives
  its own closures. The consolidated list is the one part of `FEATURES.md` that
  restates other files' state, which is why it drifted while every subsystem
  section stayed current — three closures each updated their own section and
  none walked back to §23. The preamble's "Tier 2 and Tier 3 are restored"
  note now says only Tier 3: Tier 2 has no heading because closing item 16
  retired the whole band, and a reader hunting a heading that was deliberately
  removed reads it as the same editing-script damage the note warns about.
  Tier numbers retire on close exactly as item numbers do — now stated. The
  `FEATURES.md` footer's `0fda4f4` scrape stamp is replaced by a date: a commit
  hash there is stale on the next commit and `docs_check.py` cannot see it.
  Existence is green (562 references) — the check proves a cited line exists,
  never that it still says what it was cited for, which is the exact gap this
  pass closed by hand.
  Decided by: verify-before-claiming — a documented gap that source has already
  closed is an unverified claim with a longer half-life than any code bug, since
  the next reader plans work against it.

- 2026-07-21 — A direct-writer admin command can no longer be clobbered:
  `src/base/lock.rs` is an advisory writer lock over the data dir, held for the
  daemon's lifetime and taken by `reembed`, `compact` and `gc`, which now
  refuse while it is held and name the holder. `kern status` reports data dir,
  socket, daemon, hub and lock. Built on std `File::try_lock` rather than a new
  dependency (`fs2`/`fs4`), which costs an MSRV bump 1.82 -> 1.89 — the right
  trade for alpha, where the toolchain is 1.96 and no compatibility is carried.
  The lock is an OS file lock, so a killed holder releases it and there is no
  stale-lock cleanup path to get wrong (guarded by
  `the_lock_file_is_not_the_lock`). The daemon's own acquire is non-fatal: it
  owns the graph, so it warns rather than refusing to serve, and `kern status`
  says explicitly when a daemon is serving without the lock. Verified live
  against the exact failure of 2026-07-21 — with a daemon up, all three admin
  commands refuse by name; after SIGKILL the lock is free and `reembed`
  proceeds. Halves item 9: the one-shot write commands still open the store
  directly and reconcile through the flush guard, deliberately, because
  refusing them would make the CLI unusable whenever a daemon runs.
  Decided by: fix-the-root — the "daemon must be stopped" comment on `reembed`
  was an unenforceable precondition, and a comment is not a guard — and
  name-the-tradeoff for the MSRV bump and for the one-shots left reconciling.

- 2026-07-21 — The intake has a surface, and the retry-forever tradeoff is
  finally visible: `kern intake` / `kern intake status` prints pending (age,
  last error), quarantined and done; `kern intake drain` runs one pass
  in-process with no daemon, via a `drain_now` wrapper that shares `drain_once`
  with the daemon loop so a one-shot can never diverge from what the daemon
  would have done. Building the surface found the substantive bug: three paths
  left a delta queued and wrote **no** error sidecar — no `[reason]` endpoint
  configured, a reason model replying prose, and a transient read error — so
  the exact cases ROADMAP item 8 exists to expose showed in the report as
  ordinary "waiting". All three now record why through one `record_stuck`
  funnel. The drain reuses `cmd_ingest`'s guarded flush retry, so a running
  daemon yields a refused flush and reload rather than a clobber; the advisory
  lock that would prevent the contention outright stays item 9.
  Decided by: fix-the-root — the missing CLI was the reported symptom, the
  unrecorded failures were the reason the intake was invisible in the first
  place.

- 2026-07-21 — The walk pays: ROADMAP item 86 closed with bounded
  source-weighted traversal credit in `expand` — each examined edge credits its
  far endpoint `source_score × edge_evidence` (once per edge-endpoint, summed,
  ×`traversal_credit_weight` 2.0, capped at `traversal_credit_cap` 0.5), and
  the total is clamped just below the strongest voucher's walk score so a
  neighbour never outranks what vouched for it. Root cause under the max-score
  symptom: `link_entities` scored deliberate links by `cosine(from, to)`,
  guaranteeing an edge between dissimilar texts — the one case where the edge
  is the only evidence — was the weakest edge in the graph. Deliberate links
  now carry asserted confidence (CLI user 1.0, MCP agent 0.95); auto similarity
  edges keep measured cosine. Measured on the instrument: all 8 linked pairs
  reach the top 5 (strict xfail removed), exact-match probe holds rank 1,
  recall 0.9306 / 0.9722 / 0.9471 against the 0.9167 / 0.9722 / 0.9462 master
  baseline. Judged against re-measured master, not the item's stale 0.9583 bar,
  which predated the answer-leg removal. Co-equal pooling stays rejected — the
  clamp exists because unclamped credit measured 0.7639 on recall@1.
  Decided by: fix-the-root (the link-score semantics, not another weight on a
  starved signal) and verify-before-claiming (every variant swept and measured;
  the stale baseline caught by re-running master before judging).

- 2026-07-21 — Access stamping obeys `[heat]` config: `commit_access` /
  `commit_access_ids` read `HeatConfig::default()` for half-life AND
  deposit_access, so a configured `[heat]` section (or preset) was silently
  ignored exactly where retrieval deposits heat. `HeatConfig` is now threaded
  through `query`/`query_profiled`/`query_locked` and the CommitAccess tick
  task (`ctx.heat_cfg` already existed). Behavior-neutral for the running eval
  campaign — live config and e2e both run defaults. Decided by: fix-the-root.

- 2026-07-21 — MCP ingest `kind` arg dropped, not honored: it was validated
  but never stored (kind always derived from clamped confidence), absent from
  the advertised tool schema, and its only passable value (`claim`) is the
  default — Fact is unreachable for agents (conf clamped to 0.95 <
  FACT_CONFIDENCE) and Document/Question/Conclusion are internal-only. The
  field, its dead validation path (`validate_kind`, `InternalKind`) removed;
  kind stays derived-only at the agent boundary.
  Decided by: avoided-question-first — the arg promised a choice the
  boundary never grants.

- 2026-07-21 — Alpha means no compatibility: recorded in `AGENTS.md` and every
  backward-compat implementation removed in one sweep. Gone: `kern migrate` and
  `src/base/migrate.rs` (legacy shard → LMDB import), the `.kern` file-shard
  loader (`load_legacy_dir`/`load_kern`/`save_kern`, quant sidecar,
  `sweep_stale_tmp`, base `PersistError`), `KernPreMass` and the
  `StoredKernV1`/`V2` mirrors, the pre-V4 bare-entity cold-row decode,
  `migrate_root_id` and `backfill_created_at` load-time backfills, the
  `vec_f64_compat` serde shim (vectors now serialize as native f32 on disk and
  gossip wire), and the `descriptor` serde alias on `DirectJob.hint`. The store
  now writes and decodes exactly one version, `FORMAT_V5` — any other version
  byte is a clean `BadVersion` rejection, so pre-V5 stores (and every V1-V4
  store) must be wiped and reingested; gossip peers must run the same build.
  A third scrape (agent-verified) caught the structural stragglers:
  `EntityKind` reclaims discriminant 4 (the "4 was Answer" gap held open for
  numeric-kind stability), the gossip enum comment now states the real wire law
  (serde encodes declaration order, not repr values), `QuestionPayload.from_id`
  dropped (written, never read — its consumer was removed) shrinking
  `BroadcastQuestionFunc` to three args, `retrieval.heat_half_life_secs`
  dropped (write-only duplicate of `heat.half_life_secs`), the bare
  `"initialized"` pre-spec MCP spelling dropped, the MCP query `kind` filter
  fixed to accept the stable lowercase labels (the schema advertised `"normal"`
  — a pre-rename label no parser accepted — and the serde derive wanted Rust
  variant names; both were wrong), `valid_at`/`scheme` added to the advertised
  query schema, and the graviton `rm` alias documented in its schema enum.
  Stale docs corrected: README's pre-V4 stampless-cold-row "gap" paragraph
  (that tolerance is gone), FEATURES' `V1..=V4` format claims, and two ROADMAP
  proposals that prescribed the removed `serde(default)` evolution mechanism.
  Also kept after verification: `CrdtTarget::Statements` no-op arm (in-code
  security decision: statement text only via content-addressed EntitySync),
  `traversal_count` + `CrdtTarget::ReasonTraversalCount` (tied to the OPEN
  traversal-credit roadmap item, not compat), the `-d/--daemon` flag
  (load-bearing: `spawn_daemon` launches `kern --daemon`), and the
  restart-verdict Hold for daemons predating the identity handshake (the
  documented live-handshake exception).
  A second scrape removed the residue: every `#[serde(default)]` on
  `Entity`/`Reason`/`Kern`/`CrdtDeltaPayload` (appended-field compat markers,
  inert under positional bincode — these types are never JSON-decoded), the
  `stale_digest_keys_are_ignored_not_fatal` config test (pinned tolerance for
  removed `kern.toml` keys), and the append-only-bincode law in SPECIALISTS.md,
  superseded by the single-version law: one decodable format, schema changes
  bump it. The kept hub fallback is renamed "direct path" in code, logs, and
  docs so it stops reading as compat debt.
  Kept deliberately: that direct-connect fallback in `mcp_cmd.rs` (resilience
  when the hub cannot start, not format compat) and the tolerant `HealthRes`
  decode in `trnsprt` dto plus the hub's `idle_ms == 0` distrust (both serve
  the live attach → stale-detect → auto-restart handshake against a
  still-running older daemon).
  Decided by: name-the-tradeoff (compat code and its test surface traded away
  against losing every pre-V5 store; alpha has no users to strand, so the
  simpler single-format codebase wins). Supersedes the shard-migration decision
  that introduced `kern migrate`.

- 2026-07-21 — Graviton routing measured dead and recalibrated. A 40-claim
  labelled corpus against qwen3-embedding:0.6b showed intended
  claim-to-graviton cosine distances of 0.29-0.69 while the acceptance
  midpoint sat at 0.25 ((0.15+0.35)/2) — routing required cosine similarity
  >= 0.75, which no real match reaches, so 100% of ingest fell to `generic`
  and the kern tree never formed. Three changes, each carrying its number:
  (1) radii 0.15/0.35 -> 0.35/0.75, midpoint 0.55 — admits real matches,
  still rejects off-topic (measured >= 0.57) with margin; existing kerns keep
  their stored radii until reseeded. (2) Multi-line graviton seeds are now
  example statements, embedded per line and mean-pooled: median intended
  distance 0.39 vs 0.55 for an abstract description and 0.55-0.61 for the
  same examples embedded as one concatenated blob — pooling separate embeds
  is the win, concatenation muddies it. (3) Routing ties inside the inner
  radius (probability saturates at 1.0) now break by effective distance, so
  mass stays meaningful when several gravitons fully accept. Named honestly:
  even calibrated, best-match accuracy tops out ~60-70% on entangled
  categories (decisions vs architecture); a sibling-bucket landing is
  acceptable, off-topic-in-generic is preserved, and the retrieval eval
  remains the instrument that will judge whether routing quality matters.

- 2026-07-21 — Roadmap item 34, mitigated rather than closed: the `Question` path
  gets a per-origin budget. Answering tells a peer we hold something above the
  resolve threshold for a vector THEY chose — a membership oracle that leaks
  without the content ever being sent, and it had no limit of any kind.

  `src/gossip/rate.rs` gives each origin 30 questions a minute, refusals counted
  and throttle-logged. The table is bounded and reclaims expired buckets before
  evicting live ones, because `origin` is an attacker-chosen string and the table
  is therefore itself a memory target — the same class as item 35.

  **Said plainly because the difference matters:** this makes bulk extraction
  expensive, it does not close the oracle. A patient prober still learns what we
  hold, and a peer that rotates its self-declared `origin` gets a fresh budget
  each time. Refusing outright needs an authenticated identity, which is item 33.
  Eviction prefers the oldest bucket over refusing new origins, so a spoofing peer
  cannot lock every real one out — trading a weaker bound for availability, both
  of which are inherent until identities exist.

  **Decided by:** name-the-tradeoff.

- 2026-07-21 — Roadmap item 37, its cheapest half: the gossip heartbeat stops
  deep-cloning the whole corpus to send 32 rows. `start_entity_sync` cloned every
  local entity, sorted the lot, and truncated — O(N) clones plus O(N log N) per
  heartbeat, under the graph read lock, for a payload of fixed size. `hottest_local`
  selects over references and clones only the winners: linear, using the same total
  comparator, so the chosen set and its order are unchanged.

  The batch size stays hard-coded at 32 and is now labelled as such rather than
  read as tuned. With no divergence estimate there is nothing to tune it against,
  and batch size belongs to the anti-entropy question (item 36).

  Writing the helper introduced a latent panic — `select_nth_unstable_by(n - 1)`
  underflows at `n == 0`, unreachable at the shipped batch size but not a thing to
  leave in a general helper. Guarded, with a test.

  The rest of item 37 is untouched and still listed: no per-peer rate limit, no
  divergence field, and the write-lock starvation from the four `all_ids()` loops
  in `handle_crdt_delta`.

  **Decided by:** fix-the-root.

- 2026-07-21 — Roadmap item 27, second of its four costs: `HnswIndex::delete`
  stops scanning the whole arena per victim. It scrubbed inbound edges by walking
  every node and every layer, once per delete, so a GC sweep was
  O(victims x nodes x edges).

  Deletion is now two steps: mark the node dead — searches skip a `None` node, so
  it is invisible immediately — and queue the slot; one scrub pass then clears
  every slot deleted since the last one. A sweep pays one pass instead of V.

  The safety argument is the reason it is staged rather than made cheaper in
  place. A slot enters the free list only *after* its pass, so nothing can be
  handed a slot while edges still name it — the aliasing the old comment warned
  about. Using symmetry instead would have been wrong: insert links both ways, but
  pruning an over-cap neighbour drops its back-edge while the forward edge
  remains, so a node's own layers are not a complete list of who points at it. A
  scrub driven from them would have missed exactly those edges, silently.

  `len()` had to change with it — it derived liveness from the free list, which a
  pending slot has not joined, so a deleted node would have counted as present
  until the next insert. An existing cascade test caught that.

  **Decided by:** verify-before-claiming. Both tests were confirmed to fail against
  a neutered scrub, and `recall@1`/`recall@5`/`MRR` are unchanged — an ANN index
  change that moved recall would be the failure worth catching here.

- 2026-07-21 — Roadmap item 27, one of its four costs: the cold tier stops paying
  a full-table decode per eviction. `cold_cap` sorts by age, which means decoding
  every row, and `cold_spill` called it after every single put — so at the steady
  state the tier is *designed* to sit in (full), each eviction re-decoded 50k rows,
  and a GC sweep evicting V victims paid V passes.

  `cold_cap_amortized` trims only once the tier runs a slack margin past the cap,
  then cuts all the way back to it: one pass per 1024 spills rather than one per
  spill. Direct `cold_cap` callers are untouched, so anything asking for a hard
  trim still gets one.

  **Named tradeoff:** the tier may hold up to `max + 1024` rows between passes —
  2% over a 50k cap. That is a disk bound rather than a correctness boundary, and
  it buys roughly a 500x reduction in decode work on the sweep path. If the cap
  ever becomes a hard limit, the trigger is the one line to change.

  Three of item 27's four costs remain: victim selection is still O(entities) per
  kern per sweep, `cold_search` is still a brute-force scan, and `HnswIndex::delete`
  is still O(nodes x edges) per victim.

  **Decided by:** name-the-tradeoff.

- 2026-07-21 — Roadmap item 8, first half: a stuck intake delta now says why.
  `220af94` made the reason model's prose replies retry forever rather than
  silently archiving the capture — the right side of that trade, and stated at the
  time to be acceptable only while the retrying is *visible*. It was not: the
  failure reached a tracing warning inside the daemon and nowhere else, so a delta
  that never drains looked exactly like one not yet picked up.

  The last error is now written to `<intake>/errors/<name>.txt` on failure and
  removed on the next success, and `intake_status::scan` reports pending (with age
  and last error), failed, and done counts. A stale error beside a file that has
  since succeeded would be worse than none, so clearing is wired into `archive`,
  the one path every success takes.

  The sidecars live in a subdirectory on purpose: `drain_entry` guards on
  `is_file`, so a directory is skipped, while an error file sitting in the queue
  itself would be read back as a delta and ingested. There is a test for exactly
  that, because it is the kind of mistake that only shows up as mysterious extra
  claims.

  **Owed, not claimed:** the `kern intake` subcommand and its one-shot drain. Both
  need a `Commands` variant, and `src/commands.rs` is being restructured by
  concurrent work; adding an enum arm mid-restructure trades a merge conflict for
  nothing. Item 8 stays open for that half.

  **Decided by:** fix-the-root.

- 2026-07-21 — Roadmap item 16: `commit_access` is rate-limited, closing Tier 2.
  Retrieval stamps every delivered result, so a caller replaying one query pumped
  that thought's access count *and* its heat without bound — both ranking signals,
  and "retrieval learns from use" has to mean sustained use rather than repetition.
  A thought is now reinforced at most once a minute. Adopted on paper in
  `docs/kern/stigmergy-self-improving.md`'s failure-mode table and never scheduled.

  Both stamping paths already funnelled through `stamp_access`, so the limit lives
  in one place; it returns whether the stamp took, which also stops
  `commit_access_ids` pushing a gossip delta for a suppressed increment.

  **Named tradeoff:** genuine rapid re-reads inside the window count once. That is
  the intent — heat's half-life is measured in days, so a minute of resolution
  costs nothing real, and the alternative is a signal any caller can mint. A
  rewound clock is deliberately *not* treated as throttled: heat decay already
  handles skew, and freezing the counter there would trade one bug for another.

  **Decided by:** fix-the-root.

- 2026-07-21 — Tier 0 is closed and the roadmap's own structure is repaired.
  Item 1 sat in two places at once — open at the top of the ranked list and closed
  in the appendix — which by this file's rules means the decision was never made.
  It was: the instrument exists, `e2e/` is it, and the Tier 0 block was the stale
  copy. Removing it releases what it gated: items 32, 54, 55 and all of tier 8
  stop being "unjudgeable until a metric exists" and become "apply it, measure,
  keep it if `recall@1` holds" — the loop item 86's two candidate fixes already ran.

  Repaired in the same pass: two tier headings this session destroyed. The script
  that closes an item cuts from its `###` to the next one, which swallows any
  `# Tier` boundary sitting between — so closing item 12 took Tier 2's heading with
  it and closing item 17 took Tier 3's. The items survived and silently appeared to
  rank under Tier 1. Both restored, item 16 moved back under Tier 2, and the damage
  named in the file's own preamble, because a missing heading is invisible: nothing
  looks wrong, the plan just quietly says something else.

  **Decided by:** the oracle. Two content files disagreeing — here, one file
  disagreeing with itself — is not a state to tidy away, it is a decision that was
  never recorded.

- 2026-07-21 — Roadmap item 5: an in-memory kern no longer reads as a durable
  one. Spill-before-drop holds only while a store is bound — with none, `cold_spill`
  is skipped and the victim is removed, which is the intended memory bound and not
  a bug, but `README.md` stated the guarantee unconditionally and nothing counted
  the loss. Both fixed: the README says which deployment the guarantee covers, and
  `unspilled_drops` joins the other degradation counters on MCP, the RPC DTO and
  `kern health`.

  Same standard as item 7, and the first draft of its test failed it: the test
  passed a closure of its own into `evict_victims` and asserted that closure ran,
  proving nothing about production. It now drives the real `run_gc`.

  **Decided by:** verify-before-claiming.

- 2026-07-21 — Roadmap items 7, 13, 14, 15 and 17: the fail-open paths are
  countable, an unauthenticated peer can no longer reach a local row, and an
  expired claim stops ranking.

  **Item 7.** Fail-open is the policy — a session always proceeds. Invisible
  fail-open is the defect, because a no-op is indistinguishable from a correct
  empty answer. All four paths it named now carry an unconditional counter, a
  throttled log and a health field on all three surfaces: chunks lost to a dead
  embed endpoint, entities GC cannot age because their timestamp is in the future
  (which stalls compaction indefinitely, since nothing else bounds the hot graph),
  a delivery that bypassed `min_deliver_score` because nothing cleared it, and new
  remote ids refused at the 50k ceiling while known ones keep merging. That last
  one warned once per dropped entity — a gossiping peer would have flooded the log
  it is meant to be visible in, the third instance of that pattern in one day.
  `kern health` prints its `degraded:` line only when something actually degraded.

  **Item 13** was not one bug, and "scope it to `remote-*`" would have been the
  wrong blanket fix. Reaching local rows is *intended* for the G-Counters: ids are
  content hashes, so the same fact is a local row on both nodes and slot-max is
  what makes access counts converge. The two LWW targets buy federation nothing
  there, so `ValidUntil` and `ReasonScore` are confined to `remote-*` while the
  counters keep their reach — their real exposure is attacker-chosen slot names,
  which needs authenticated identity (item 33) and is ranking inflation, not truth
  corruption.

  **Item 14.** The comment above `handle_entity_sync` read "Content↔id binding is
  NOT verified". That binding is why merge is safe as set-union and why a peer
  cannot alter text you hold. Bodies are hash-checked on receipt — but only ids
  *shaped* like a content hash are judged, because dropping legitimate remote
  knowledge is worse than the exposure, and the invariant was verified on the real
  creation path rather than assumed.

  **Item 15.** `handle_pulse` defaulted an unknown kern id to the LOCAL ROOT, so a
  peer sending garbage deposited heat straight into it, with no bound on strength.
  No design intent justified the fallback.

  **Item 17** had to follow 13, and the roadmap said so. Enforcing `valid_until`
  while LWW deltas could still reach local rows would have armed a remote
  expire-any-local-claim attack repo-wide. It is now enforced on every retrieve,
  and deliberately skipped when the query names an instant of its own — a
  point-in-time query judges validity AT that instant, so a since-expired claim is
  exactly what it should return.

  **Decided by:** fix-the-root, and verify-before-claiming for the test standard.
  Every guard here has a paired test asserting the healthy path does *not* trip
  it, and each was confirmed to fail against the pre-change code. Item 17's call
  site has its own test, because the predicate tests pass unchanged when the call
  is deleted — which is precisely how `valid_until` came to be honoured by a
  function nothing invoked.

- 2026-07-21 — Roadmap item 86 gains a measured cause and loses a guess. The
  instrument's first finding was that a reason edge changes no ranking. Two causes
  were isolated; one is fixed.

  Fixed: `expand` pruned a neighbour scoring below `best_seed * decay`, comparing a
  seed's pure query cosine (up to 1.0) against a neighbour score whose ceiling is
  `w.reason + w.edge = 0.30` for a neighbour the query does not match directly.
  Measured 0.2411 against a 0.2500 bar — pruned by 0.0089, with `chains` empty. The
  bar now comes from the best score seen among *neighbours*.

  Open, and the obvious fix is measured wrong: `visited` allows one pop per entity
  and `results` keeps the max, so a neighbour that is already a content hit keeps
  its seed score. Pooling the evidence instead moves all 8 test pairs — 5 into the
  top five — and drops the exact-match probe from rank 1 to rank 3, which
  `e2e/test_retrieval.py` caught. The cause is structural: the best-matching entity
  pops first, so it can only *give* hop evidence, never receive it, and any
  co-equal pooling penalises the best answer.

  **Decided by:** name-the-tradeoff. Recording why the plausible fix is wrong is
  worth more than the one rank the partial fix bought, and it is what stops the
  next attempt repeating it.

- 2026-07-21 — `docs_check.py` validates cross-doc line anchors. It checked line
  numbers for `src/*.rs:NNN` but not for doc-to-doc citations, so `ROADMAP.md`'s 24
  `FEATURES.md:NNN` anchors rotted silently when that file shifted ~380 lines. Both
  the `docs/…` and bare-sibling forms now fail on a line past EOF. The limitation
  is unchanged and still printed in the tool's own output: an anchor in range but
  pointing at the wrong line still passes.

  The audit it enabled found `architecture.mdx` listing four fail-open paths as
  "still silent" that item 7 had just made countable, four `src/*.rs:NNN` anchors
  pointing at unrelated lines, and — worst — `e2e/test_invariants.py`'s xfail
  reason still recording the pre-fix multi-hop measurement, which four doc pages
  then cited for a figure it contradicted. Fixed at the source.

  **Decided by:** verify-before-claiming.

- 2026-07-21 — `kern reembed` now restamps the store when it completes. Found
  switching this box to all-granite: `check_embed_stamp` deliberately never
  adopts a new identity on mismatch (a config swap must not rewrite the record
  of what produced the stored vectors), but `reembed` relied on exactly that
  path — it rewrote every vector, then saved under a stamp check that refuses
  to update, so `health` kept reporting MISMATCH forever after a successful
  re-embed. The completed re-embed is the one legitimate transition, so it now
  calls `set_embed_stamp` explicitly, and only on full success — a failed cold
  tier keeps the old stamp precisely so `health` keeps accusing until the
  re-run. Regression test drives the whole command against a stamped store and
  a fake embed endpoint. Reproducing this also demonstrated ROADMAP item 9
  live: a hub respawned by an MCP proxy mid-reembed flushed its stale
  in-memory graph over the direct write — "daemon must be stopped" is an
  unenforceable comment while `hub.auto_start` defaults true. Recorded on
  item 9 rather than patched here.
  **Decided by:** fix-the-root. The root is the missing stamp transition in
  the one command whose job is that transition — not a looser stamp check,
  which would reopen the silent-swap hole the stamp exists to close.

- 2026-07-21 — Live-testing hot reload uncovered and fixed the graph-wipe bug
  shipped in `248722f`'s idle sweep. Kill chain, each link verified in
  isolation: (1) `is_idle(None) == true` — but `last_access: None` describes
  EVERY kern on a freshly booted daemon, so the first maintenance beat
  idle-unloaded the entire loaded graph ~60s after boot; (2)
  `evict_empty_children` read the unloaded child's resident-map miss as "does
  not exist" and deregistered it; (3) `deregister` deletes the on-disk row.
  Loaded-from-disk graphs died within two ticks; graphs still in RAM from
  their own ingests never did, which is why the bug survived e2e (fresh
  stores) and killed the repo's own 42-thought dogfood store. Fixes:
  `is_idle(None) = false` (unknown is not idle — a kern earns idleness from a
  real access clock), `evict_empty_children` skips unloaded children via the
  new `GraphGnn::is_unloaded`, and two load-path hardenings from the same
  hunt: `graph_from_store` now errors (`StoreError::RootMissing`) instead of
  silently returning an empty graph stamped with the store's live epoch when
  rows exist without a root, and `load_graph`'s empty fallback logs at error
  level instead of silently absorbing. Regression tests pin all four.
  Verified end-to-end: restart on a populated store + tick + hot reload +
  successor tick, entities and disk rows stable throughout.

- 2026-07-21 — One word per classification axis: "descriptor" retired. The
  name covered three unrelated things — the claim-type label distill emits,
  the free-text chunking context on ingest, and a root registry nothing read.
  Now: `Claim.kind` + `claim_kind` tool/CLI + `root.claim_kinds` for the type
  axis; `hint` for the prompt-context axis (ingest param, worker job field,
  `split.rs`); gravitons untouched (topic axis). The registry stops being
  dead: registered kinds are offered to the distillation LLM alongside
  `DEFAULT_KINDS` and accepted by `parse_claims` (`spawn_intake` passes a
  kinds closure into `intake::run`). Supersedes the "descriptor" vocabulary
  everywhere (tool name, CLI subcommand, health stat key, MCP resource URI —
  now `kern://local/claim-kinds`). **Decided by:** fix-the-root — the fix is
  the taxonomy, not a doc note atop the ambiguity. Tradeoffs named: breaking
  rename of the MCP tool (`descriptor` → `claim_kind`), CLI subcommand and
  health JSON key; the ingest param keeps a `descriptor` serde alias and
  pending direct-intake JSON decodes via the same alias, but external agent
  configs calling the old tool name must update. Kern shard bincode is
  positional, so the `Kern.claim_kinds` field rename is wire-compatible.

- 2026-07-21 — **V1 carries no answer compatibility.** The two shims kept
  during the answer-leg removal earlier today are deleted: the `[answer]`
  config tombstone (an unknown section is now just unknown — pre-1.0, nobody
  has that config), and the dead `EntityKind::Answer` variant. Verified safe
  against live stores before removal: no code path ever constructed an
  `Answer` entity — nor, it turns out, a `Question`/`Conclusion` entity; only
  Fact/Claim/Document rows exist on disk, all below the removed serde index,
  so decoding is unaffected and the append-only bincode law is not violated,
  merely unexercised. Discriminant 4 is annotated retired in the enum so it
  is not blindly reused. **Decided by:** the oracle, on the user's ruling —
  this is a v1 implementation; compatibility surface starts at v1.0,
  not before.

- 2026-07-21 — **The answer leg is deleted; retrieval is the only read
  path.** `query` returns scored passages, enriched edges, and path chains —
  no synthesis, ever. The calling agent synthesizes, which is what agents are
  for: measured today, granite4:3b's synthesized answer confabulated over a
  correct top-3 retrieval, the same failure that got the LoCoMo bench deleted
  ("answer quality set the ceiling"). Removed with it, because they only ever
  ran when `answer=true` and would otherwise be dead code behind silently
  no-op knobs: HyDE, the LLM rerank, and the semantic query cache — the read
  path is now LLM-free end to end, the exact path the e2e instrument scores.
  Also gone: the `[answer]` config role (refused at load with a removal
  notice), the `--answer` CLI flag, the answer/streaming machinery and CPU-pin
  logic in `llm.rs` (reason no longer competes with an interactive model for
  GPU), and `answer_max_facts`/`answer_abstain_hint`/`hyde_*`/`rerank_*`/
  `query_cache_*` knobs. `retrieval/answer.rs` renamed to
  `retrieval/query.rs`. Kept deliberately: the `Answer` entity kind (persisted
  data, append-only law), gossip's question/answer protocol (federation
  edges), and UNTRUSTED tagging — now applied to *delivered* chains and
  remote ids, since the trust boundary moved to the synthesizing caller.
  Closes ROADMAP items 63, 68, 80; halves item 66. Verified: 747 lib tests,
  e2e floors unchanged through the removal (recall@1 0.9167, recall@5 0.9722,
  MRR 0.9462) — proof the scored path never used the LLM.
  **Decided by:** the user, and fix-the-root over patching the answerer: a
  small local model synthesizing above a big calling model was an
  architecture error, not a prompt problem. Supersedes the answer role
  sections of "Lifecycle freshness ships" and every `answer=true` surface.

- 2026-07-21 — Presets become the whole tuning surface, and the default is
  `relaxed`. Supersedes the same-day entry below on two points. (1) The
  overlay-under-scopes design ("any explicit key still wins") is replaced:
  `Preset::apply` (`src/config/preset.rs`) is now the only writer of heat
  half-life, dedup threshold, and retrieval breadth, and the `[heat]`,
  `[ingest]`, `[retrieval]` sections are refused at load with a pointer to
  `preset` — a loud error over a silently-ignored key, consistent with the
  exit-78 boot gate. The per-knob escape hatch is deleted deliberately:
  thirty knobs nobody can defend individually are the configuration surface
  kern exists to not have. (2) The default preset is `relaxed` (30d
  half-life, 0.98 dedup, seed_k 25 / 800 expansions / 40 results / 8 answer
  facts) — for a memory tool, losing a fact costs more than surfacing a
  noisy one, so keep-more is the posture a configless user should land on.
  **Decided by:** name-the-tradeoff. Named: (a) knob sweeps for tuning
  experiments now require a code change, not a config edit — acceptable
  because the e2e instrument drives built binaries anyway; (b) the LoCoMo
  baseline (0.137, 2026-07-20) was measured on the medium-era defaults,
  and configless runs now exercise `relaxed` — the next instrument run must
  either pin `preset = "medium"` for comparability or re-baseline, tracked
  in ROADMAP item 87.

- 2026-07-21 — Config presets ship: a top-level `preset =
  "relaxed"|"medium"|"tight"` key picks a whole memory posture in one line
  instead of hand-tuning ~30 retrieval/heat/ingest knobs. Implemented as an
  overlay layer UNDER the config scopes (defaults < preset < user < project,
  `src/config/preset.rs`), so any explicitly set key still wins and the
  layering machinery already tested in `io.rs` carries it. `medium` is the
  empty overlay — the shipped defaults ARE medium, so the two can never
  drift. Relaxed: 30d heat half-life, 0.98 dedup, broader search, 40
  results / 8 answer facts. Tight: 3d half-life, 0.90 dedup, narrower
  search, 12 results / 4 answer facts. Unknown names refuse to load, same
  as any invalid config. The `setup` MCP tool now offers the tiers to the
  wiring agent.
  **Decided by:** name-the-tradeoff. The tradeoff, named: relaxed/tight
  values are hand-picked judgment, not eval-measured — the LoCoMo
  instrument has only run against medium, and medium stays byte-identical
  to the baseline defaults precisely so the presets can ship without
  invalidating it. Measuring the outer tiers is a `ROADMAP` question, not a
  blocker. Supersedes nothing; before this the only options were defaults
  or per-knob surgery.

- 2026-07-21 — Lifecycle freshness ships: identity handshake, client-side
  auto-restart, Unix hot reload, and the `setup` MCP tool. Trigger was a
  measured dogfooding outage: the repo's own daemon ran 36 hours against a
  dead LLM endpoint because `.kern/kern.toml` was written after boot and
  nothing ever rereads config — every shipped fix was invisible to the running
  process. Root cause is staleness with no detector, so the fix is an identity
  the client can compare: `build_id` fingerprints the executable (len+mtime,
  not semver — all dev builds report the same version; not path — `cargo
  install` hardlinks `target/release`), `config_id` hashes the resolved
  config, both ride `HealthRes` append-only. `kern mcp` compares on attach and
  gracefully replaces a mismatched daemon; tradeoffs named: a 15s uptime floor
  stops two differing builds restarting each other forever, empty ids (older
  daemons) are Hold not Stale, and every failure falls open to proxying.
  Hot reload closes the other half — a daemon that outlives its binary: the
  daemon polls its own path and hands the bound socket to a freshly spawned
  successor as an inherited fd (backlog holds connects during the successor's
  boot; the proxy reconnects and retries once). Measured handover 39ms.
  Windows gets no fd handoff; auto-restart covers it. The hub reaper also now
  drops nodes whose root directory vanished (seven 20h-old e2e hub orphans
  observed with deleted cwds). The `setup` tool is the agent-agnostic
  installer decided over per-host plugins: one instruction set returned to
  the calling agent (current [done]/[todo] state, graviton seeding, capture
  rule for the host's instruction file, verify loop) — kern never writes a
  host's config; the agent does the wiring. MCP surface is now twelve tools.

- 2026-07-21 — **kern has an instrument again.** Item 1 asked what measures
  retrieval quality with no LLM in the scoring loop. The answer is `e2e/`, grown
  rather than replaced: `fake_llm.py` already served deterministic feature-hashed
  bag-of-words embeddings — real cosine, identical every run, no GPU — and
  `test_retrieval.py` already asserted "the matching fact ranks first". That was
  a retrieval-quality assertion with no model in the scoring loop, sitting
  unrecognised.

  `e2e/test_recall.py` adds the number: 36 facts across unrelated topics, 72
  paraphrase probes, scored `recall@1` / `recall@5` / `MRR` against floors set
  from the measured value. First run and every run since: **0.9583 / 1.0000 /
  0.9792**, bit-identical. `e2e/test_invariants.py` adds the properties — each
  named for the `VISION.md` criterion it defends.

  This dissolves the five sub-questions rather than answering them. Ground truth
  is written by the test, so there is no labelled corpus, no LoCoMo answer key,
  and — decisively — no dependency on the turn-level claim provenance that does
  not exist and nobody had scheduled. Ingest non-determinism is out of the loop
  because the tests ingest directly. Synthesis is out of scope: the fake answer
  model echoes its prompt, so a test can assert the retrieved context *arrived*,
  never how it read.

  **Named up front, not discovered later:** the fake embedder is bag-of-words
  hashing, not a semantic model. This measures kern's machinery — fusion,
  expansion, ranking, dedup, supersede, heat — and says nothing about how a real
  embedding model behaves. That is the price of having no model in the loop, and
  it is the right price. Second limitation, equally on the record: the floors
  make this a *regression detector*. It can say kern got worse. It cannot say
  kern is good, and no number here is comparable to anything Zep or Mem0
  publishes. `VISION.md`'s "no quality claim without an instrument" is now
  satisfiable for retrieval and still unsatisfied for answering.

  It earned itself immediately. The multi-hop lead — previously smoke, n=8 — is
  now **measured**: linked and unlinked pairs rank identically to four decimals,
  reproduced by an independent harness. `kern link` contributes nothing at
  retrieval. Recorded as `xfail(strict=True)` so it flips loudly when fixed.
  Two invariants could not be written at all: `supersede` and `as_of` are
  unreachable from the CLI, MCP only, so they are `skip` markers naming the
  missing surface rather than fake coverage.

  Items 71/72/73 land with it, because an instrument nothing runs is not an
  instrument: CI gains a lint job running `just check` (fmt and clippy were
  enforced by memory alone) and an e2e job, and `.pi/update.sh` — which was
  gitignored, so the fresh-checkout guarantee it was supposed to provide did not
  exist — is now tracked as the single exception to the `/.pi/` ignore.

  **Decided by:** verify-before-claiming. Also fixed here because it made the
  whole gate theatre: the banned-vocabulary CI step could never fail. It grepped
  a gitignored path, and GNU grep returns 2 for a missing path *even when it
  matched*, so the `if` never fired on a runner.

- 2026-07-21 — Roadmap items 2, 3, 4 and 6: the tick survives a panic, the
  embedding-model swap is caught, `as_of` stops lying over the cold tier, and
  cold evictions are counted.

  **Item 2.** `tick::start` was one bare `tokio::spawn` with no `catch_unwind`
  and a dropped `JoinHandle`, so one panic ended decay, GC, persist, clustering
  and the idle sweep for the process lifetime with nothing logged — and
  `parking_lot` does not poison, so the graph was left half-written and still
  readable. The loop is now guarded, the fault counted, and the log says plainly
  that the kern's state may be partially written.

  Containment alone would have been a trap, and review caught it: the first
  landing removed the `.expect` panics in the GNN forward path by returning zero
  tensors, which converted a loud crash into `apply_gnn_updates` persisting
  corrupt weights — a silent wrong answer, the exact class this tier exists to
  delete. The chain is now fallible end to end (`Model::forward`/`backward`
  return `Result`), a failed propagation writes nothing, and the per-matmul
  `error!` that would have emitted ~50 lines per task became one at the
  propagation boundary. Degradation is surfaced on all three surfaces — MCP, the
  RPC DTO (appended, back-compat test extended) and `kern health`.

  **Item 3** was the one nearly claimed falsely. The first landing added
  `check_embed_stamp`, `set_embed_stamp` and `query_dim_ok` with **zero non-test
  callers**, feeding health fields hardwired to read "healthy" on a store that
  had exactly the problem. Now bound at every graph open, stamped on flush, and
  guarded on the hot query path — fail-open and counted, per repo policy, not an
  abort. Two follow-ups review then caught: the guard was O(all entities) per
  query memoized on a key every `get_mut` invalidates, making ingest quadratic;
  and `kern reembed --embed-model X` stamped the *configured* model over vectors
  built by X, so health reported the wrong dimension and a later open said
  "Match", masking the swap the stamp exists to catch.

  **Item 4.** `valid_from`/`valid_to`/`invalidated_at` are `#[serde(skip)]` and
  survived only through `StoredKern.temporal`, so a cold-recovered revision came
  back with all three `None` and `is_valid_at` answered true at every instant —
  point-in-time queries exact over the hot graph and silently lossy over the cold
  tail. The triple now round-trips; legacy rows still decode.

  **Item 6.** Cold eviction past 50k is the intended memory bound and stays; it
  was simply invisible. Counted, surfaced in health, and logged once per sweep
  rather than once per row.

  **Decided by:** fix-the-root. Owed and recorded rather than claimed: the
  infallible `GraphLayer`/`BackwardGraphLayer` impls are now test-only and want
  deleting, and a permanently-failing kern is still re-enqueued every tick with
  no backoff — visible via the counter, not yet suppressed.

- 2026-07-21 — A config scope that redirects an endpoint no longer inherits that
  endpoint's key. This is the tradeoff of the same day's deep-merge decision,
  found after the fact rather than named before it, which by `name-the-tradeoff`
  makes it a defect and not a note. Under the old section-replace semantics a
  project config that set `[embed] url` wiped the user's `[embed] key` as a side
  effect, and that accident was load-bearing: with deep merge, a `.kern/kern.toml`
  committed in any repo you clone can point `[embed] url` at its own endpoint and
  receive the `sk-live-…` you set globally on the first embed call. `reason_key`
  and `answer_key` fall back to `embed.key` (`src/config/mod.rs`), so redirecting
  any single endpoint reaches the same secret. For a project whose first claim is
  local-first, zero-egress, that is the wrong direction to fail in.

  So `src/config/secrets.rs` seals it: a scope that sets a section's `url` does
  not inherit that section's `key`, and must supply its own or go without.
  Deliberately narrow — a project setting only `model` still inherits the user's
  url and key, which is the whole point of layering. Its own file rather than a
  branch inside `merge_deep`, because deciding what a scope may inherit is not
  the same job as merging two tables.

  **Decided by:** name-the-tradeoff, and fix-the-root over the alternative of
  documenting the hazard in `configure.mdx` and leaving the behaviour. Supersedes
  the unqualified deep merge recorded in the same day's items 10/11/12 entry.

- 2026-07-21 — Roadmap items 10, 11 and 12: the detached daemon gets a log, an
  invalid config stops startup, and config scopes deep-merge per key.

  **Item 10** is the one that makes the rest of tier 1 observable. `spawn_detached`
  put `Stdio::null()` on all three fds, and with hub auto-start shipped that is
  the *default* posture — so every silent defect items 2 through 7 describe is
  invisible in exactly the configuration most people run. Captured output now
  appends to a per-arg `hub.log` / `daemon.log`, owner-only, created on demand,
  never truncated, because a restart must not erase the log explaining why it
  restarted; a log that cannot be opened falls back to `/dev/null` and says so on
  the parent's stderr rather than costing the spawn. **The first landing fixed the
  wrong path** — adversarial review caught that `spawn_daemon` is only the legacy
  fallback, and the default hub-first route (`kern mcp` → `attach_via_hub` → hub
  → `src/hub/node.rs`) still nulled all three fds, so the process that actually
  runs ingest, tick and retrieval was still silent. Both paths now share
  `src/config/detached_log.rs`. `log_dir` also moved inside `data_dir` rather than
  above it: taking the parent put `daemon.log` directly in `$HOME` for any
  relocated store.

  **Item 11.** `main.rs` logged a warning on a failed `validate()` and continued
  with whatever parsed, and `unwrap_or_default()` discarded genuine parse and IO
  errors on top. The repo's fail-open policy is scoped to intake and recall — a
  session must always proceed — and says nothing about booting on settings known
  to be wrong; continuing there is not failing open, it is failing silently, which
  is the disease this tier exists to cure. Both now print the offending key to
  stderr and exit 78 (`EX_CONFIG`). An *absent* config stays legitimate and still
  defaults silently; `--help` and `--version` still answer under a broken one.

  **Item 12.** `merge_sections` inserted each top-level key wholesale, so a project
  config setting `[reason] model` destroyed the user's `[reason] url` and `key`.
  The doc comment admitted it and a test asserted it, which made it an unrecorded
  decision rather than an oversight. Now `merge_deep` recurses wherever both
  scopes hold a table; arrays stay leaves, verified rather than assumed —
  `watcher.roots` and `gossip.peers` are complete lists whose empty case is
  meaningful, not accumulators. The test asserting the old behaviour was rewritten
  in place, not deleted.

  Also corrected in the same pass, since both said the opposite of the code the
  moment it landed: `configure.mdx`'s "Precedence is per-section, not per-key"
  callout and its "a config that fails validation does not abort startup" line.
  `docs_check.py` cannot catch that class — it proves a citation exists, never
  that it still says the thing.

  **Decided by:** fix-the-root. Known-owed and recorded rather than claimed:
  `serve.mcp_addr` exists as a config field with no reader (`src/commands.rs`),
  `num_ctx`/`keep_alive`/`num_gpu` cannot warn because they are constants in
  `src/llm.rs` and not config keys at all, and `src/commands/admin.rs` still
  swallows a foreign root's config error on the merge path.

- 2026-07-21 — `docs_check.py` now scans every documentation directory, not two
  of four, and three claims it immediately caught are struck from `FEATURES.md`
  and `SPECIALISTS.md`. The tool checked `docs/site/content/**.mdx` and
  `README.md`; `docs/kern/` and `docs/oracle/` — including the roadmap that
  ranks the work and the feature list that says what exists — were unverified,
  so the two files whose whole job is to describe reality were the two nothing
  checked. Extending the scan to 876 references found that `FEATURES.md` claimed
  a `Dropout` layer at `src/gnn/dropout.rs` and a whole `locks` feature at
  `src/base/locks.rs` with `read_recovered`/`write_recovered` wrappers "that
  survive a poisoned lock" — none of which exist in any form, and the last of
  which is doubly false since `parking_lot` does not poison at all — and that
  the federation specialist's scope cited `src/wire.rs`, deleted long enough ago
  that gossip types now live in `src/gossip/types.rs`. Also corrected:
  `docs/kern/diskann-disk-index.md` cited `src/base/cold.rs` for a cold tier
  that was absorbed into `src/base/store.rs`; the claim it makes is still true,
  only the path had moved.

  The extension needed one thing the old tool had no concept of: a citation that
  is *supposed* to name a file that is gone. A changelog entry recording a
  deletion must cite the deleted file, and demanding it resolve would mean
  rewriting the record to satisfy the checker. So `CHANGELOG.md` carries a
  `<!-- docs-check: historical -->` marker and is skipped whole, and a single
  line naming a deletion is excused in place — which lets a present-tense file
  like `ROADMAP.md` say what it removed without either lying or being exempted.
  Both escapes are pinned by `--selftest`, because an escape nobody tests is a
  hole nobody sees.

  **Decided by:** verify-before-claiming. A citation nobody checks is a claim
  nobody verified, and the two files the repo trusts most to describe itself
  were the ones with no instrument pointed at them. The tool's limitation is
  stated in its own output rather than left implicit — it proves a cited line
  exists, never that it still says the thing. Supersedes the previous scope of
  `scripts/docs_check.py` and roadmap item 74.

- 2026-07-21 — The roadmap is re-founded as one importance-ranked list, and the
  documentation is reconciled to it. `ORACLE.md` says `ROADMAP.md` holds
  "decisions ahead, **ordered**"; what it held was nine topical sections grouped
  by subsystem, inside which nothing said what came first. A reader could learn
  everything that was open and still not learn what to do next, which is the one
  question a plan exists to answer. Items are now numbered 1 to 85, item 1 being
  the most important open thing in the repo, importance falling monotonically
  from there. Rank is assigned by severity × reach, with sequencing constraints
  as hard edges — where B cannot precede A, A ranks above B and says so in both
  entries. Tier headings survive as commentary explaining why a band sits where
  it does; the number is the plan. Context that is not work — north star,
  competitive position, non-goals, repo laws — moved below the list, and the
  `[x]` entries moved into a "Closed and verified — do not re-open" appendix,
  since a file whose stated job is "what is left" should not open with what is
  not.

  The reorder was funded by a three-way audit run in parallel over disjoint
  ground — the docs site, the `FEATURES.md`/`README.md`/`docs/kern/` set, and
  `src/` — because ordering work by importance is worthless if the inventory is
  wrong. It was wrong in both directions. **Roughly forty items were appended
  that no plan had ever held**, most of them documented on the site or in
  `FEATURES.md`'s own gap blocks as known limitations and funded nowhere: a
  panicking tick task that silently kills all maintenance for the process
  lifetime (no `catch_unwind`, dropped `JoinHandle`, and `parking_lot` does not
  poison, so the graph is left half-written and still readable — now item 2); a
  fail-open policy with no error surface anywhere; a PageRank power iteration per
  query; three more superlinear costs inside one GC sweep; an unauthenticated
  membership-probe oracle on the `Question` path; namespace rotation as unbounded
  storage; peer authority never built, so the Sybil defences have no signal to
  weight; and the process gaps that let all of it rot — the oracle's own
  pre-commit hook is untracked with no installer, so a fresh clone enforces
  nothing, CI runs neither `fmt` nor `clippy`, and `docs_check.py` scans two of
  the four documentation directories, leaving `docs/oracle/` and `docs/kern/`
  citations unchecked. **Four items were factually wrong and are corrected in
  place, not deleted:** HNSW has no tombstones to compact (`delete` recycles
  slots via a free list; the real cost is an O(nodes × edges) scan), there is no
  second typed transport surface to kill (`kern_rpc` is a generic envelope with
  no DTOs to overlap), standalone `kern mcp` does run its maintenance tick and
  lacks only gossip, and there is no port-clash validation in `config/serve.rs`
  for the hub's phase 3 to collapse.

  Reconciled in the same change, at the source of each false claim rather than at
  the pages that echoed it: the site still published withdrawn LoCoMo and
  retrieval-bench figures on two pages, still described the prose-archiving
  data-loss bug fixed the same day in `220af94` — including as a troubleshooting
  diagnostic that now points the opposite way, since the post-fix symptom is a
  stuck intake rather than a fast-draining empty one — still told MCP callers to
  "ingest it as a `Fact`" on a path that clamps confidence below the 1.0 `Fact`
  needs, still advised seeding a graviton with a whole document when documents
  truncate at the embed context window, and still called a re-run seed "close to
  idempotent" against a cosine dedup that any paraphrase evades.

  The same sweep over the rest of the documentation: `README.md` still declared
  the Delta, Question and Pulse senders dead and the fetch RPC unwired — all four
  are live, and what is actually missing is anti-entropy — still pinned the
  version at 1.0.0 against `Cargo.toml`'s 1.1.0, and still stated `as_of`,
  spill-before-drop and coordinator-free convergence unqualified, each of which
  this file funds a defect against. `VISION.md`'s opening paragraph still said
  the daemon "takes in durable facts from sessions on its own" — the exact clause
  its own test criterion was reworded away from on 2026-07-20 — and its claim
  criterion still gated on "the recorded baseline", an artifact withdrawn the
  same day; the criterion now gates on the standard that exists. `FEATURES.md`
  carried a *plan* for the replacement eval harness, which repo law 4 reserves to
  this roadmap and which pre-answered an open sub-question of item 1; the plan is
  stripped and only the present-tense facts remain. It also under-reported the
  MCP surface as ten tools when eleven exist (`move` was in no table anywhere but
  the site), omitted `Entity.acl` from a field list claiming to scrape everything
  that exists, and still ranked a query-cache improvement retired in July. In
  `docs/kern/`, five research notes contained execution plans, migration stages
  and phase orderings while the directory's own README declared it holds "never
  plans"; each is converted to a record of how the design decomposes, with
  scheduling pointed at roadmap items by title, and four claims stale against
  current source are corrected — no `PnCounter` exists, `Delta` has a live
  sender, OR-Set-for-`statements` was reversed rather than deferred, and PQ is a
  non-goal rather than "the next step". Seven dead relative links across those
  notes were repaired or dropped. `docs_check.py` passes at 561 references — and
  note that it verified none of `docs/kern/` or `docs/oracle/`, which is the
  roadmap item "`docs_check.py` checks two directories out of four".

  Tradeoffs, named. **Item numbers churn on every re-rank**, so every citation
  from another document must point at an item by title, never by number — the
  same drift-bait argument that removed a baked reference count from
  `FEATURES.md`. **A flat list loses subsystem grouping**; track tags plus the
  fact that dependencies force related items adjacent are the mitigation, and
  they are weaker than sections were. And the ranking makes one bet explicitly:
  **federation's security holes sit below the default-path defects because gossip
  is off by default** — an unauthenticated peer that can LWW a local row is worse
  in kind than a silently-lying `as_of`, and rarer in practice by the ratio of
  users who turn gossip on. If that default ever flips, tier 2 and tier 5 move
  above tier 1 and this entry is the record of why they were not there first.

  Decided by: avoided-question-first (the ordering *is* the avoided question — a
  file listing sixty open items with no rank has answered everything except which
  one to do), verify-before-claiming (every unchecked item was re-checked against
  `src/` before being ranked, which is what surfaced the four wrong ones; no item
  is ranked on a document's word), fix-the-root (each contradiction corrected in
  the file that originated it, so the pages that repeated it stop repeating it),
  name-the-tradeoff (numbering churn, lost grouping, and the gossip-off bet
  stated above rather than discovered later), parallel-independent-work (three
  audits over disjoint file sets, because a single pass over four documentation
  trees and a Rust source tree is one context that runs out). Supersedes the §1–§9
  topical structure and the "ordered by leverage" note that formerly stood in for
  a ranking.

- 2026-07-21 — A prose-answering reason model stops silently discarding a delta.
  `distill` returned `Some([])` both when the model emitted a well-formed empty
  JSON array (genuine "nothing worth keeping") and when it replied in prose with
  no parseable array (a weak model ignoring the format) — the intake archived the
  delta in both cases, so a real conversation the model failed to structure was
  lost with no error, on the primary ingest path. `parse_claims` now returns
  `Option<Vec<Claim>>`: `None` when no JSON array parses (prose or malformed
  span), `Some(vec)` when an array parses even if it filters to empty. `distill`
  propagates that, so a format failure takes the existing retry path (`distill`
  → `None` → `extract_claims` → `None` → delta left queued) instead of archiving.
  A genuine `[]` still archives. The two prior tests that asserted the buggy
  archive-on-prose behavior are corrected, and
  `prose_reply_carrying_knowledge_is_not_lost` guards the regression. 72 ingest
  tests green, clippy clean. Tradeoff, named: a model that *persistently* answers
  in prose now retries forever, identical to a persistent LLM outage — chosen as
  the safe side (queue, never lose), and made visible once the `kern intake`
  status/drain item lands.
  Decided by: fix-the-root (the ambiguity was one overloaded `Vec::new()` return,
  split at the source rather than sniffed downstream), verify-before-claiming
  (contract traced through `intake.rs` before changing it; the genuine-`[]`
  archive path re-confirmed by test), name-the-tradeoff (infinite retry on a
  persistent prose model stated, not hidden). Closes the ROADMAP §6a silent-loss
  item.

- 2026-07-21 — Automatic session capture is declared a non-goal, and the docs are
  reconciled to the two entry points that actually exist. kern has exactly two
  caller-driven ways to write: an agent calls MCP `ingest` (verified wired,
  `mcp/tools_mutate.rs:153`) — the primary path — or a transcript is dropped into
  `.kern/intake/`, which the daemon drains and distills (verified wired,
  `spawn_intake` at `commands.rs:632`, `intake::run` at `intake.rs:220`). Neither
  is broken; both were confirmed in source before any wording changed. What never
  existed as a shipped feature is *automatic* session capture — the old Claude
  Code Stop hook (deleted `483b37c`) is not being restored, because that would
  undo the agent-agnostic decision. `VISION.md`'s first test criterion, which
  read "intake is a byproduct of working / no manual ingestion step," asserted an
  auto-capture kern deliberately does not do; it is reworded to the two-entry
  contract. Five site pages plus `README.md` that implied automatic capture
  (`index.mdx`, `concepts/architecture.mdx`, `concepts/acceptance.mdx`,
  `howto/memory-bank.mdx`, the two "automatic loop" `Next` links, and the
  intake-recall troubleshooting node that told users to "check your client hook"
  when none ships) are reconciled to it. The two `ROADMAP.md` §6 items that
  tracked "automatic session intake has no producer" as open work are closed by
  this decision, not by building a producer; the `kern intake` status/drain item
  survives on its own merit, no longer a prerequisite for a producer that will
  never be built.
  Decided by: fix-the-root (the false promise lived in the VISION wording, so the
  wording is corrected, not just the pages that echoed it), name-the-tradeoff
  (zero-touch capture is given up; an honest, agent-agnostic, two-entry contract
  is gained — any harness can write the drop dir, and no shipped hook ties kern
  to one agent), verify-before-claiming (both live entry paths were traced to
  their wiring in source before the docs were touched). Supersedes the "intake is
  a byproduct of working" criterion and the "no producer" ROADMAP items.

- 2026-07-20 — Cleanup sweep after the docs reconciliation. The e2e harness's
  only third-party dependency is now declared (`e2e/requirements.txt`, recipe
  `just e2e-install`): `just test` runs `pytest -q e2e` and nothing in the repo
  said where pytest comes from, so a clean clone could not run its own test
  suite. Recorded alongside it, not fixed: **the e2e suite is not run in CI at
  all** — `ci.yml` runs `cargo test --workspace` only, leaving the sole
  end-to-end exercise of the real binary local-only and free to rot. Wiring it
  is a CI-minutes-versus-coverage call left to a human (ROADMAP §6,
  `FEATURES.md` §21a). Also: five pointers to `docs/FEDERATION-SECURITY.md`
  re-aimed at the site's Security page now that the former is a stub (README,
  `why-kern.mdx`, `SPECIALISTS.md`, `FEATURES.md`, and the pagerank research
  note); three concept pages that sent readers to GitHub research notes now
  link the in-site Decisions pages that supersede them; the pagerank note's own
  "rate clipper that once existed was later removed" line — the sentence that
  produced the withdrawn regression finding — corrected at the source; and a
  reference count baked into `FEATURES.md` removed, since a number that changes
  on every doc edit is drift-bait of exactly the kind this work exists to stop.
  Decided by: fix-the-root (correct the note that caused the error, not just
  the pages that repeated it). Verified: build artifacts and `__pycache__` are
  ignored and untracked, no dangling references to the deleted mkdocs, bench or
  eval surfaces remain, and the struck `§3A` citations are re-pointed.

- 2026-07-20 — The "Sybil defence regressed" finding is withdrawn: nothing
  regressed, because nothing was ever wired. User-prompted re-check of the
  commits. Two Sybil-resistance components were fully written and never
  connected — `RateClipper` (`gossip/sybil.rs`, 175 LoC), whose `set_clipper()`
  has no call site in *any* commit (`git log -S"set_clipper("` returns only the
  commit that defined it and the one that deleted it), and
  `trimmed_mean_merge_hits` (`gossip/merge.rs`, 241 LoC), documented in its own
  comment as "a Sybil-resistant alternative" for fusing per-peer hit lists, also
  callerless. `dc02a18` deleted both in an audited dead-code sweep that traced
  every deletion to zero live callers; runtime behaviour was unchanged because
  neither had ever run. Decided by: verify-before-claiming — the prior entry
  inferred a regression from a doc sentence ("a rate clipper that once existed
  was later removed") without reading the commit, and "once existed" turned out
  to mean "compiled", not "protected anything". Consequence is the opposite of
  alarming: this is unbuilt work with a working reference implementation in git,
  a cheaper starting point than assumed. Corrected in `ROADMAP.md` §5,
  `decisions/pagerank-authority.mdx` and `decisions/knowledge-not-gradients.mdx`
  (which also called trimmed-mean "not built" when it was built-then-deleted).
  Supersedes the "A Sybil defence regressed" item recorded earlier the same day.

- 2026-07-20 — `ROADMAP.md` is reconciled against source and the docs site, and
  the eval section is withdrawn rather than migrated. A gap reconciliation
  across all 25 site pages found the plan itself was the least accurate document
  in the repo. Five items described shipped work: the Pulse and Question senders
  are live (`commands.rs:900-930`, `tick.rs:64`), the query cache already matches
  paraphrases by cosine ≥ 0.97 (`retrieval/cache.rs:60`), `validate_kind` and
  `validate_conf` are both called (`mcp/tools_mutate.rs:115-117`), and
  `FEATURES.md` still called the fetch RPC dead. The "highest-leverage safety
  fix in the repo" — forgeable `Fact`/`Superseded` — was wrong on all three
  counts: `Superseded` is an `EntityStatus` and was never an `EntityKind`, and
  `kind` is derived from clamped confidence, so the MCP caller's `kind` is
  discarded outright. Worse, §1's north star and all five of §3's items were
  scheduled against the LoCoMo harness and baseline deleted in `8d8b19e`, three
  commits after the file's own stamp. Those are struck, not re-pointed, because
  `8d8b19e` measured why: a grounded run with kern bypassed scored 0.187 where
  kern scored 0.027, so the answerer set the ceiling and the number could not
  steer memory work. Published figures are **withdrawn, not superseded** — the
  claim standard is now "no quality claim of any kind" until a retrieval-only
  metric (recall@k / MRR / NDCG, no LLM in the scoring loop) is decided, which
  is recorded as §3's open question with three sub-questions and no deciding
  behavior yet. Decided by: verify-before-claiming, and the oracle's own rule
  that content files disagreeing means the decision was not made in either
  version. Also funded, having been documented on the site but tracked nowhere:
  five silent wrong-answer/silent-loss defects now leading §6 (cold eviction
  drops the temporal side-map so `as_of` lies; a prose-answering reason model
  archives deltas having stored nothing; an embedding-model swap zeroes recall;
  in-memory mode drops without spilling; the cold tier is a lossy FIFO past
  50k), plus unverified entity content against claimed ids — which undercuts the
  content-addressing invariant every other federation guarantee rests on — a
  regressed Sybil rate clipper, and the operational, belief-model and retrieval
  residuals. `concepts/stigmergy.mdx` filed its own gaps into `FEATURES.md`;
  corrected, since FEATURES is not a plan (repo law 4).

- 2026-07-20 — The gossip delta-scoping gap is split by target, and enforcing
  `valid_until` is sequenced behind it. An earlier entry in this same session
  prescribed "confine deltas to `remote-*` kerns"; reading the code, that fix
  is wrong for the counters — ids are content hashes, so the same fact is a
  local row on both nodes and `retrieval/score.rs:255` emits access deltas for
  local entities precisely so G-Counter slots merge across replicas. Blanket
  scoping would kill intended federation. Split instead: the two LWW targets
  (`ValidUntil`, `ReasonScore`) confine to `remote-*` now, needing no wire
  change; the counters' real exposure is attacker-chosen replica-slot names,
  which genuinely gates on authenticated peer identity. This subsumes open
  decision (a) — LWW-vs-max-join for `Reason.score` is no longer only a
  trust-signalling question once an untrusted writer can reach a local row.
  The coupling that makes it urgent: `matches_filter` honours `valid_until`
  only when a caller passes `valid_at` (`retrieval/score.rs:168`), so §6's
  "enforce `valid_until` in retrieval" would arm a remote
  expire-any-local-claim attack on the default path — that item is now marked
  blocked on this one. Also recorded: `handle_pulse` falls back to `g.root.id`
  on an unknown `kern_id` (`gossip/handler.rs:319`), so a peer needs no ids at
  all to heat your root kern. Decided by: verify-before-claiming (the earlier
  prescription was asserted from a summary, not from the handler; reading it
  reversed the conclusion). Supersedes this session's "fix is scoping, not
  crypto" note in ROADMAP §5.

- 2026-07-20 — `install.sh` and `install.ps1` are restored from `e46c859^`, and
  the docs checker grew a check that would have caught their loss. They were
  deleted on 2026-06-16 as collateral in a commit about OpenAI-compatible LLM
  paths (same sweep as `benches/`, `test_utils/`, `viewer/`), which the commit
  message never mentions — so for a month both the README and the site told
  every new user to `curl` a URL that 404s. The restore is verbatim, not a
  rewrite, because `release.yml` never stopped publishing the artifacts the
  scripts consume: `kern-<target>.tar.gz` / `.zip`, and all fourteen target
  triples the scripts detect still appear in the release matrix.
  Decided by: fix-the-root (the dead link is the symptom; the missing file is the cause,
  and an unenforced convention is why it stayed missing). `scripts/docs_check.py`
  now also resolves links into this repo's own files on GitHub — verified to
  fail on exactly this regression — and covers `README.md`, not just the site.
  Its workflow drops its path filter entirely: the check costs two seconds and
  every filter is a way for a deletion to slip past.

- 2026-07-20 — Documentation becomes a code-verified contract, not prose.
  Three moves in one change. (1) Security gets a first-class in-site page
  (`concepts/security.mdx`) covering the whole posture — Unix-socket RPC and
  its `0600` window, stdio-vs-HTTP MCP, plaintext LMDB, LLM-endpoint egress,
  and the federation trust model with CAN/CANNOT tables — and
  `docs/FEDERATION-SECURITY.md` collapses to a pointer, because two copies
  meant the drifted one was the one operators read. (2) A `Decisions` section
  ports the `docs/kern/` research notes into indexed pages that state the
  alternatives rejected and why, each re-verified against source. (3) The
  `src/…:line` citations those pages depend on are now enforced:
  `scripts/docs_check.py` + `.github/workflows/docs-check.yml` fail any push
  or PR whose code change orphans a doc reference.
  Decided by: verify-before-claiming (a doc citing a line that no longer exists is a
  claim nobody checked; CI now checks it). Tradeoff, named: write-time
  friction on every code change that moves a cited line, bought against
  readers acting on stale security and merge-semantics claims between manual
  audit sweeps — for a system whose docs state exact CRDT bounds, the
  friction is the cheaper side. The checker proves a cited line exists, not
  that it still says the same thing; semantic drift still needs audit.
  Supersedes `docs/FEDERATION-SECURITY.md` as the security source of truth.
  Audit against source in the same change corrected the doc claims that had
  already drifted: `statements` never merges (content-addressing forbids it)
  rather than "set union", the fetch RPC is wired and live rather than dead,
  and remote heat/access/confidence are stripped at the merge boundary rather
  than max-joined from attacker values.

- 2026-07-20 — The in-binary docs viewer (`kern docs`, `src/docs/`, ~316
  LoC embedding docs/site markdown via `include_str!`) is deleted. It broke
  when the markdown moved into the fumadocs app as MDX, and the published
  site plus `/llms-full.txt` supersede an in-terminal copy. User-directed.
  Decided by: fix-the-root (delete the duplicate doc surface rather than
  re-point the embeds at MDX they can't render). Tradeoff, named: no offline
  docs in the binary; docs now live only at the site and in the repo's
  `docs/site/content/`.

- 2026-07-20 — Docs site moved from mkdocs (terminal theme + custom TUI
  overlay) to fumadocs with the stock UI, static-exported for GitHub Pages.
  User-directed platform choice; stock-over-custom UI confirmed by the user
  when offered a reskin. Decided by: name-the-tradeoff. Tradeoff, named: the
  TUI identity (keyboard nav, gruvbox, Departure Mono) and the Python docs
  toolchain die; gained MDX, client-side Orama search, `llms.txt` /
  `llms-full.txt` generation, and a Node toolchain in CI. Deploy still
  targets the `gh-pages` branch, so no Pages settings change. Four dead
  `howto/benchmark.md` links (page never existed) de-linked in passing.
  Supersedes the mkdocs setup (`mkdocs.yml`, `docs/overrides/`,
  `docs/requirements.txt`, `docs/site/assets/`).

- 2026-07-20 — Hybrid seed fusion rescores its survivors by query cosine
  before expansion. RRF's reciprocal-rank scores (~1/rrf_k) replaced the
  seeds' cosine scores, while `expand()` scores neighbours on the cosine
  scale — `merge()` pooled the two, so any expanded neighbour outscored
  every seed and hybrid/query ranking inverted (the e2e harness's strict
  xfail, now a hard regression test: cosine-0 entities ranked above the
  token-overlap match). RRF now decides only WHICH entities seed; cosine
  against the query decides how much they score. Decided by: fix-the-root
  (the scale mismatch, not the symptom ordering, was repaired). Tradeoff,
  named: a BM25-strong but embedding-weak match now enters expansion with
  its true (low) cosine — lexical strength buys seed-set membership, not
  final magnitude; accepted because magnitude faked from rank order is what
  produced the inversion.

- 2026-07-20 — Chunk external ids are keyed on the full source identity
  (`source_id()` + chunk index), not the bare section, and CLI `kern ingest`
  derives its inline source hash from the text instead of the constant
  `"user"`. Both fed the supersede path with colliding external ids, so chunk
  0 of every document (watcher files have empty sections) and every CLI
  ingest silently superseded the previous one — data loss the new e2e
  harness surfaced on its first run. An identity-less source now gets an
  empty external id and never supersedes. Decided by: fix-the-root.
  Tradeoff, named: entities persisted under old-format external ids no
  longer match, so the first re-ingest after this change creates a new
  entity instead of superseding the old one (cosine dedup still catches
  identical text); accepted as a one-time discontinuity against ongoing
  silent loss.

- 2026-07-20 — E2E lives in a Python pytest harness (`e2e/`, `just e2e`):
  it drives the real binary against a deterministic fake Ollama server
  (`fake_llm.py` — feature-hashed embeddings, echo answers), covering
  answer retrieval end to end (ingest → search/query → answer prompt), with
  the hub supervisor suite folded in as `test_hub.py`. The Rust
  `e2e/hub_supervisor.rs` target and its `Cargo.toml` `[[test]]` block are
  deleted as superseded. Decided by: verify-before-claiming (retrieval
  behavior needs a runnable proof, not an asserted one; pytest is the premade
  harness for it, and the Rust target it replaces goes in the same change).
  Tradeoff, named: `just test` and
  `just e2e` now need a Python with pytest; accepted because the retrieval
  tests need a scriptable fake LLM server, which Rust dev-deps made
  heavier, not lighter. A known query-mode
  ranking gap (expansion + boosts flatten seed similarity on small graphs)
  is pinned as a strict xfail in `test_retrieval.py`, owned by the refit
  campaign. Supersedes: the 2026-07-20 `tests/` → `e2e/` rename entry's
  explicit `[[test]]` target.

- 2026-07-20 — `src/config/wsl.rs` and the `Config::load` loopback rewrite are
  deleted. Resolving a WSL2 NAT loopback URL to the Windows host gateway is
  environment plumbing, not kern's job: kern now uses configured URLs verbatim,
  and a WSL NAT user pins the gateway in `kern.toml` (docs updated with the
  one-liner to find it). This supersedes the 2026-07-17 auto-rewrite; if an
  outside tool is ever wanted, it wraps kern rather than living inside it.
  Tradeoff, named: the zero-config promise regresses on WSL2 NAT — a fresh
  install there sits silently on a dead loopback again until the URL is pinned;
  accepted because two 300 ms TCP probes and a route-table parse inside config
  load was kern guessing at the network, and a wrong guess is worse than an
  explicit setting. Also: `.pi/` added to `.gitignore` (local tooling layer).
  Decided by: delete-superseded (in-binary URL guessing superseded by explicit
  config plus documentation).

- 2026-07-20 — `scripts/insight.py` and the `just insight` recipe are deleted;
  `scripts/` is gone. It printed a repo snapshot (build, tests, LOC, oracle
  counts) but nothing consumed it — no CI job, no hook, no doc — and `git`,
  `cargo` and `tokei` already report everything in it except the oracle
  counts, which are a `grep` away. It was repaired hours earlier the same day
  (it had been silently reporting zeros after the governance files moved);
  that repair is superseded by removing the thing entirely, which is the
  cheaper end state for a tool with no consumer. It is not moved into `e2e/`:
  that directory is for suites that drive the real binary, not for
  reporting. Decided by: delete-superseded (an unused reporter is cruft
  however correct it is). Supersedes: the insight.py path repair recorded
  immediately above.

- 2026-07-20 — `tests/` becomes `e2e/`, and the target is declared explicitly
  in `Cargo.toml`. The directory holds exactly one suite, and it is not a unit
  test: it spawns the real `kern` binary over a real Unix socket under a
  private `XDG_RUNTIME_DIR`. `e2e` says that; `tests` implied the 708
  in-`src` unit tests lived there too. Tradeoff, named: `tests/` is Cargo's
  auto-discovered path, so renaming it would silently stop the suite from
  running — hence the explicit `[[test]]` target. `CARGO_BIN_EXE_kern` is
  still provided for declared targets, which is what lets the suite drive the
  binary, and is also why this suite cannot move into `src/`: a unit test has
  no way to locate the built binary. Verified: `cargo test --test
  hub_supervisor` still lists all 6.
  Also repairs `scripts/insight.py`, which had been reporting **zero** active
  features, decisions and specialists ever since the governance files moved to
  `docs/oracle/` — it read them from the repo root and a missing file counted
  as 0 rather than erroring. It now resolves either location and prints an
  explicit NOT FOUND line when a file is genuinely absent; a plausible-looking
  zero is worse than an error in a script whose whole premise is that every
  number came from a run. Its dead `section_bench()` (probing for the
  benchmark doc deleted earlier today) is removed.
  Decided by: fix-the-root (silent-zero, not just the path), name-the-tradeoff
  (auto-discovery lost, declared target gained), delete-superseded (the bench
  probe). Supersedes: `tests/` as the directory name and the root-relative
  oracle paths in insight.py.

- 2026-07-20 — The digest is deleted. `.kern/digest.md`, `build_digest`, the 30s
  rebuild loop, and all five `[intake]` digest knobs are gone; recall is now
  `query` only. It dumped a `heat * conf_mean` ranked slice of the graph into a
  file that sessions read wholesale as this node's own knowledge — with no
  provenance a reader could weigh, ranked by popularity rather than by relevance
  to the session at hand, duplicating what `query` already does with provenance
  intact. The one thing it did that `query` cannot — answer before the agent
  knows what to ask — did not justify injecting unvetted claims into every
  session. `ROADMAP.md` had already recorded the doubt ("still written every 30s
  and nothing reads it") without acting on it.
  What this costs, named: the architecture's "the recall hook never opens the
  store" property is gone. A session-start hook could previously get context with
  zero dependencies — no daemon, no embeddings, no MCP. Recall now requires a
  live `query`. Accepted: a dependency-free path that injects unattributable
  claims is worse than a dependency that returns attributable ones.
  Consequence worth stating plainly: the `remote-*` exclusion added earlier today
  disappears with the module. That is correct rather than a regression — it
  guarded provenance-free wholesale injection, and the surface no longer exists.
  Federated claims stay reachable through `query`, where they are marked
  untrusted. Config compatibility is pinned by test, not asserted: `Config` and
  `IntakeConfig` are `#[serde(default)]` with no `deny_unknown_fields`, so a
  stale `[intake]` block carrying the five dead keys parses and its live keys
  still apply.
  Decided by: delete-superseded, name-the-tradeoff, avoided-question-first (the
  roadmap had flagged this and left it standing).

- 2026-07-20 — Peer text is marked untrusted where the LLM reads it, and cannot
  be the majority of the evidence. The synthesis prompt fed retrieved passages to
  the answer model with no trust marking at all — the largest remaining injection
  surface after the reranker was bounded. Facts and chain nodes now carry the
  same `UNTRUSTED` tag `rerank` established (tagging only facts would have left
  "get your text into a chain instead" as a trivial bypass), the preamble is
  emitted only when peer text is actually present so an all-local prompt stays
  byte-identical, and `admit_facts` caps remote passages at the local count.
  The zero-locals case is exempt, mirroring the stance `apply_remote_trust`
  already takes: remote stays reachable when it is the only match. Full
  exclusion-when-locals-suffice was rejected — one weak local hit would suppress
  genuinely better federated knowledge for a marginal security gain.
  `remote_ids` was computed only when `cfg.rerank_enabled`; that gate is deleted
  rather than widened, because a trust signal conditional on an unrelated feature
  flag is itself the defect. Cost measured, not assumed: the always-on scan sits
  inside the noise band of the gated runs, path stays sub-ms.
  Found while there and fixed: `build_digest` iterated remote kerns, so peer text
  landed in `.kern/digest.md` and was injected into new sessions **as this node's
  own memory**, unmarked — strictly worse than the surface being fixed, and the
  digest has nowhere to put a trust tag, so remote kerns are excluded there
  outright.
  Named for the next reader, because this must not read as solved: the tagging is
  a SOFT defense. A single remote passage still carries an injection; the cap
  bounds volume, not effect. There is no structural analogue to the reranker's
  tier sort, because synthesis emits free-form prose rather than an ordering.
  The attribution instruction is unenforced. Chain text is bounded by
  `ANSWER_MAX_CHAINS`, not by the fact cap. All of it is downstream mitigation of
  the root cause: federation is unauthenticated.
  Decided by: name-the-tradeoff, fix-the-root (the gate deleted, not widened),
  verify-before-claiming (cost measured; no recall claim made — see below).

- 2026-07-20 — No retrieval-quality claim accompanies the above. The LoCoMo eval
  and retrieval bench were deleted in `8d8b19e`, so `just bench-workload`, the
  recall@10 1.0000 / NDCG@10 0.9993 baselines, and the recorded LoCoMo baseline
  are unmeasurable in this tree. The claim standard requires a run, and there is
  no harness to run; the retrieval-facing risk is instead bounded by
  construction — no scoring, ordering or delivery logic changed, and the
  all-local prompt is asserted byte-identical.
  Decided by: verify-before-claiming (absence of a measurement is recorded, not
  papered over).

- 2026-07-20 — A duplicate detected at accept time is merged, not dropped. The
  threshold unification earlier today narrowed this loss window without closing
  it: `commit_entity` still returned `deduped: true` having stored nothing AND
  merged nothing — no `observe_support`, no `Rephrase` edge — so the
  corroboration signal was discarded and the alternate phrasing lost. It still
  bit after unification because the two checks run different queries:
  `find_duplicate` searches `entity_idx` alone while `is_duplicate` also searches
  `gnn_entity_idx` and blends `0.4*content + 0.6*gnn`, so they can disagree.
  The merge body is extracted as `merge_duplicate` in `base/accept.rs` with
  `update_existing_entity` reduced to a wrapper — direct reuse was impossible
  because that function takes `&Arc<RwLock<GraphGnn>>` and acquires its own write
  lock while `commit_entity` already holds `&mut GraphGnn`, so calling it would
  have deadlocked. There is now exactly one copy of merge semantics, proven by
  reverting: breaking the invariant inside `merge_duplicate` fails the
  ingest-path tests too. Two defects found while there: `AcceptResult.entity_id`
  returned the *incoming* id on the dedup path — an entity that was never stored,
  so any caller resolving it got a miss — and it now names the survivor; and the
  supersede hook was being invoked while holding the write lock, now moved out.
  Content-addressing is preserved: the merge touches only `conf_alpha` and
  `updated_at`, never `statements`/`chunks`/`vector`, pinned by a test asserting
  the survivor is bit-identical after a merge with different wording. Verified
  unreachable with remote content — every `accept` caller is local ingest, so the
  new `observe_support` is not a confidence-injection path.
  Decided by: fix-the-root (one shared merge, not a third copy),
  delete-superseded, fix-bugs-on-sight (the lying `entity_id`, the hook under
  the lock).

- 2026-07-20 — The whole eval and bench surface is deleted: `bench_support/`
  (14 modules, ~4k LoC), the `locomo_eval` and `retrieval_bench` binaries,
  the `bench` cargo feature, `traces/`, `eval/`, the `bench-workload`/`trace`
  just recipes, and the eval docs. Cause, measured the same day: the LoCoMo
  score is `ingest x retrieval x answering` collapsed into one LLM-judged
  number, and the answering term dominates. A grounded run (whole
  conversation in the prompt, kern bypassed entirely) scored **0.187** on the
  same slice where kern scored 0.027 — so the ceiling was set by a 3B
  answerer, not by memory. Worse, three eval-side prompt changes landed the
  same day (abstention hint, short-answer style, and the 5-fact answer cap
  they interact with) took the slice from a 0.131 expectation to 0.027 by
  making the model refuse 69% of ANSWERABLE probes and truncate correct
  answers past what the judge accepts. A metric that three prompt tweaks can
  move further than any retrieval change cannot steer retrieval work.
  What replaces it, deliberately not yet written: retrieval-only scoring —
  recall@k / MRR / NDCG against LoCoMo's per-turn `evidence` labels, with no
  LLM in the loop, so it is deterministic, fast, and immune to judge bias,
  model swaps, GPU contention and prompt wording. It needs turn-level claim
  provenance; ingest currently records only session granularity.
  Tradeoff, named: this drops external comparability (LoCoMo is the published
  benchmark) and discards the recorded 0.137 baseline as a live reference.
  Accepted — that number measured a pipeline whose dominant term was the
  answerer, so it was never the retrieval signal it was read as. The dataset
  (CC BY-NC, gitignored) and the three seed baselines were copied out of the
  tree before deletion rather than destroyed.
  Decided by: delete-superseded (a metric that cannot steer the work is
  cruft), verify-before-claiming (the grounded ceiling is what exposed this),
  name-the-tradeoff. Supersedes: the LoCoMo end-to-end eval as kern's primary
  quality signal, and `docs/kern/eval-locomo.md`, `bench-retrieval.md`,
  `locomo-improvements.md`, `locomo-baseline-2026-07-19.json`.

- 2026-07-20 — Remoteness defeats durable-kind immunity. `EntityKind::Fact` is
  immune to removal at three independent sites — `remove_entity` silently
  no-ops, `forget_entity` errors, and `is_cold_victim` never reclaims — and a
  federated entity arrives with an attacker-chosen `kind`. So a peer could pin
  unbounded, permanently un-deletable, un-GC-able rows in the local graph by
  setting one enum field, with the operator unable to remove them even by hand.
  A denial-of-storage vector that outlasts every ranking defense: those stop the
  rows ranking well, not existing forever. All three sites now treat a durable
  kind in a `remote-*` kern as forgettable and evictable. The bypass covers
  `Document` as well as `Fact` — a remote Document is the identical vector and
  costs no extra code. **Local Facts keep their immunity in every case**: that
  is a stated product guarantee ("Facts are never auto-forgotten", VISION and
  README), and the regression guards for it were written first and pass
  unchanged. Verified rather than assumed: `merge_remote_entity` cannot land a
  remote entity outside a `remote-*` kern — both call sites build the phantom id
  by format string, and the hijack guard blocks migration by id collision — so a
  remote Fact stays remote for life and a local Fact never sees the bypass.
  Decided by: fix-the-root (one predicate at all three gates), fix-bugs-on-sight,
  verify-before-claiming (every `is_fact` gate audited, not just the three
  reported).

- 2026-07-20 — Cruft sweep across the whole tree, driven by six parallel
  call-graph audits (base/vector, retrieval+gnn, federation, ingest+config,
  surface, bench/eval). Deleted: `trnsprt::search` (whole module, zero callers
  after its svc/mock died), the 14 unused `SOURCE_*` constants (half-abandoned
  taxonomy; `USER_SOURCE`/`AGENT_SOURCE` live on), the `ttl_secs` ingest knob
  (permanently `None` — not serde-exposed, no construction site ever set it, so
  the `valid_until` branches it fed were unreachable), GNN spares never
  constructed by the live model (`dropout.rs` + the whole `set_training`/
  `dropout_mut` machinery, `Tensor::mul` which only dropout used,
  `Activation::{Tanh,LeakyRelu,Gelu}`, write-only `Edge.features`,
  `Graph::num_edges`), `tick::pulse::should_consolidate` (dead duplicate of the
  live inline check), the discarded `let _loss` forward pass (24 wasted
  epochs/tick), the redundant `kern docs --section` flag (`section.or(page)` —
  both fed the same argument), and trnsprt's unused `tracing` dep. Demoted
  test-only or file-local `pub` items (`save_kern`, `float_cosine_distance`,
  `INT8_MAX_ABS`, `gc_empty_kerns`, `running_max`, `doc_count`, `mod docs`) and
  dropped dead re-exports (`config::DEFAULT_REASON_MODEL`,
  `config::ModeWeights`). Unified the four hand-rolled epoch-time helpers
  (`mcp::epoch_ms`, `pulse::now_secs`, two private `now_nanos`) into
  `base::util::{now_ms, now_secs, now_nanos}`. Kept deliberately, with reasons:
  LayerNorm (live — layer 1 trains with `norm=true`; the audit that called it
  dead was wrong), gossip (default-off but documented in docs/site as the
  federation surface), the legacy `.kern`-shard migrate cluster (only entry is
  `kern migrate`; it is the upgrade path for pre-LMDB data dirs — delete it the
  release after the format is considered extinct), both tokenizers (lexical
  stems for BM25, bench embed hashes unstemmed — unifying changes embeddings),
  and `eval/baseline` (gitignored local artifacts, likely A/B inputs). 710 lib
  tests green after surgery. Decided by: delete-superseded,
  verify-before-claiming (every deletion grep-verified; two audit claims
  refuted), name-the-tradeoff (migrate cluster kept, expiry named).
  Follow-up consolidation round (same day): `tool_result_from_envelope`
  (mcp_cmd) deleted — byte-identical duplicate of `mcp::value_to_tool_result`,
  which is now `pub(crate)` and owns the envelope tests; the four
  platform-split `spawn_hub`/`spawn_daemon` fns collapsed into one
  `spawn_detached(arg)` with cfg-scoped detach flags; digest's hand-rolled
  char-boundary truncation replaced with `util::truncate` (display suffix
  `…`→`...`; util kept as-is so eval-path prompts stay byte-stable vs the
  recorded baseline); the twice-repeated tool-schema deserialization folded
  into `tools::typed_tool_schemas()`. Checked and clean: retry logic is
  properly layered (`llm::is_transient` single source, embed consumes it),
  lexical poison-lock idiom appears in one file only.
  Round 3: the endpoint-override flag quad (`--embed-url/--embed-model/
  --reason-url/--reason-model`), hand-repeated across six subcommand variants
  (Ingest, Query, Search, Reembed, Link, Graviton Add) with per-arm `resolve`
  calls, collapsed into two flattened clap structs — `EmbedArgs` and `LlmArgs`
  (embed + reason) — each owning its config-fallback `resolve()`; CLI surface
  verified byte-identical via `--help`. Graviton's inline
  `as_deref().unwrap_or` fallback now routes through the same helper. Also
  removed orphaned `.splinter/src/{config-io,log,test-utils}` index dirs whose
  source crates were deleted, and repaired stale `synthesize`/
  `answer_prompt_from` test call sites left behind by the remote-trust
  signature change (missing `remote: &HashSet` arg — lib tests were not
  compiling).
  Round 4: `Client::new_embed_only` gained an `embed_key` param and became the
  single way to build an embed-only client — the three prod sites (reembed,
  query, graviton add) that hand-rolled
  `Client::new(Endpoint::default(), Endpoint::default(), …)` now call it, and
  the 20 test callers pass `""`. The two hand-rolled
  `block_in_place(|| Handle::current().block_on(…))` blocks in the MCP proxy
  now route through `llm::block_on_in_place` — one place owns the
  runtime-or-None contract (proxy maps None to an RPC error instead of the old
  panic-on-no-runtime).
  Round 5: the three copy-pasted MCP test-server builders (resources,
  tools_admin, tools_mutate — full 10-field `Server` literals each) collapsed
  into `test_support::mcp_server()`; tools_admin overrides `save_fn` on the
  shared builder for its save-counter. The duplicated `text()` JSON extractor
  became `test_support::tool_text`, aliased locally. Net ~90 lines of test
  scaffolding gone; any future `Server` field lands in one place.
  Round 6: added `GraphGnn::root_kern_mut()` (documented as no-load/no-epoch,
  unlike `get_mut`) and collapsed digest.rs's ten
  clone-root-id-then-`kerns.get_mut` couplets onto it. Audited the other 16
  root-id sites and left them: each needs the id VALUE next to `&mut g`
  (pulse calls, `Kern::new(parent)`) — the two-step is the borrow split, not
  duplication. `strip_think` and LLM-output parsing verified already
  single-sourced. Consolidation yield is flattening; remaining repetition is
  load-bearing.

- 2026-07-20 — Remote entities cast no PageRank votes, buy no kind privileges,
  and cannot be reranked above local content. Three paths the trust weight left
  open, now closed. **PageRank:** `EntityAdjacency::build` walked reasons from
  every kern, so a peer could farm edges inside its own phantom kern to inflate
  its seed rank. The obvious predicate was wrong and worth recording:
  `Reason::is_remote()` is `!to_net_id.is_empty()`, which flags an edge
  *crossing a network boundary* — a farm built entirely inside one phantom kern
  has empty `to_net_id` on every edge and reads as not-remote, so filtering on it
  would have caught nothing. Filtered on the owning kern instead; remote
  entities stay adjacency *nodes* (still rankable) but cast no votes. **Kind:**
  the fact bonus (`score.rs`) and the `important_access_threshold` bypass
  (`seed.rs`) are withheld from remote entities, while the `kind` itself is
  preserved — federated typing is the point of sharing structured claims, and
  the attack was the privilege, not the type. **Rerank:** candidates are tagged
  untrusted in the prompt AND a stable sort partitions remotes below locals after
  the model returns, so model judgment survives within a tier but no injected
  text can promote a remote above a local. Named for the next reader: the
  structural bound is airtight for ordering, but ordering *among* remotes is
  still model-controlled, and the synthesis prompt is NOT hardened — it is the
  larger injection surface and remains open. Recall@10 and NDCG@10 verified
  bit-identical against the fixes neutralized; an apparent 34% latency
  regression was chased down to cold-cache noise (medians 0.095 vs 0.095).
  Decided by: verify-before-claiming (predicate checked rather than assumed;
  bench A/B'd rather than asserted), name-the-tradeoff (residual risk stated).

- 2026-07-20 — Remote entities cannot buy rank. Federated entities land in
  `remote-*` phantom kerns but are inserted into the SHARED `entity_idx`/
  `gnn_entity_idx`, and retrieval had no remote filter — the `is_remote()` checks
  at `expand.rs:256,365` apply to reasons, not entities. So a peer could place
  attacker-chosen text with attacker-chosen ranking signals into the index local
  queries search, and rank it into what the LLM reads: a prompt-injection channel
  bounded only by the 50k phantom cap. Two defenses, because the second alone is
  insufficient and that was proven by reverting: `merge_remote_entity` now
  strips every caller-settable ranking signal (`heat`, `access_count`,
  `accessed_at`, `conf_alpha`, `conf_beta`) from an entity landing in a phantom
  kern, and `score::apply_remote_trust` multiplies the composite by
  `remote_trust_weight` (default 0.4) AFTER boosts and gravity so it binds on the
  full score. Heat is zeroed rather than clamped: a bound still lets a peer pin
  heat at the ceiling forever, whereas zero makes the rule flat and checkable —
  heat is earned by local access only. The strip is gated on the phantom-kern id
  so `absorb_graph`'s trusted disk-fold path keeps its heat. Named for the next
  reader: legitimate remote popularity no longer transfers, and the first-contact
  insert path was bypassing `merge_entity`'s confidence guard entirely — the
  vulnerability was wider than first reported.
  Decided by: fix-the-root, name-the-tradeoff, verify-before-claiming (each
  defense verified by reverting it).

- 2026-07-20 — `filter_delivery` sorts after boosts. It truncated in pre-boost
  order and nothing re-sorted when `opts` was `None`, so `apply_boosts`,
  `apply_gravity` and the new remote penalty were invisible on that path — and
  the delivery cut ignored boosts on EVERY path. A live retrieval-quality defect
  independent of the security work that surfaced it; recall@10 held at 1.0000
  against the recorded baseline after the fix.
  Decided by: fix-bugs-on-sight.

- 2026-07-20 — The public gossip seed is opt-in. `GossipConfig::seed` defaults
  to `false`, so enabling federation no longer auto-dials `seed.kern.dev`.
  Federation is unauthenticated and unencrypted, and `announce`/`entity_sync`
  push root graviton text and the hottest entity bodies in cleartext — auto-
  dialing a public host would have shipped a user's hottest memories to a third
  party on `enabled = true` alone, and handed a stranger the injection channel
  above. Explicit `seed_addr` and `seed = true` still work, and the seed can be
  disabled while gossip stays on, so an air-gapped LAN federation never phones
  out. Also fixed on the way: `Node::new` inserted the configured peer list
  verbatim, bypassing the `GOSSIP_MAX_PEERS` cap that `add_peer` enforces — a
  200-entry `peers` list already blew past it.
  Decided by: name-the-tradeoff (peer discovery traded for not calling a public
  host by default), fix-bugs-on-sight (the peer cap).

- 2026-07-20 — Queue metrics reach `health`. `record_task_latency` ran in
  production while `metrics()`/`pending_count()` were read only by tests, so
  task latency accumulated and was never surfaced. `HealthRes` gains
  `queue_depth`, `task_avg_ms` and `tasks_done`, appended with `#[serde(default)]`
  per the append-only law and pinned by a test that decodes an older payload
  without them. Named for the next reader: `task_avg_ms` is a LIFETIME mean, not
  current load — it converges and stops moving, so `tasks_done` ships beside it
  because a mean over 3 tasks and over 3 million are otherwise
  indistinguishable. No rolling window: the existing accumulator cannot support
  one without real statistics machinery, and an EWMA field is the smallest
  upgrade if recency turns out to matter.
  Decided by: name-the-tradeoff (a lifetime figure labelled as one beats a
  rolling window nobody asked for).

- 2026-07-20 — `spawn_child_clusters` migrates through `move_entity`. The
  clustering path hand-rolled entity migration and diverged from the real one in
  three ways: remove-then-check ordering (latent, since `child_id` comes from
  `spawn_unnamed_child`), it moved NO reasons — leaving outgoing edges in the
  parent while their `from` entity lived in the child, breaking the "an edge
  lives in its `from` kern" invariant — and it never called `index_entity`, so
  the entity→kern index went stale after every clustering pass. The last two were
  live, not latent: clustering runs every tick. Routed through the repaired
  `move_entity` rather than patching the copy, because two copies of migration
  logic is what produced the divergence. Named for the next reader: of the two
  tests written here, only the reason-carrying/reindex one is decisive — the
  ordering test passes against the old code too, and is kept as a contract guard
  rather than as evidence.
  Decided by: fix-the-root, verify-before-claiming (the non-decisive test is
  labelled, not counted).

- 2026-07-20 — Mutation ops share one core between CLI and MCP. `link` gets an
  extracted `graph_ops::link_entities` (find/cosine/reason_id/add_reason);
  `forget` reuses `graph_ops::forget_entity`; `graviton` list reads one
  `graviton_rows`; graviton add/remove already shared `base::accept` and
  `descriptor` is a bare BTreeMap insert/remove per side — wrapping either
  would be pure indirection, skipped. Two real drift bugs fell out: MCP `link`
  reported success with no edge added when the kern vanished during the
  LLM-call unlock window (now errors, and endpoints re-validate under the
  write lock), and MCP `forget` computed the cascade count with a raw
  subtraction where the CLI used `saturating_sub`. MCP-side policy (agent
  caller boundaries, required-field checks) stays in the tool wrappers.
  Decided by: fix-the-root (drift impossible once the copy is gone),
  fix-bugs-on-sight.

- 2026-07-20 — `base::locks` shim deleted. parking_lot never poisons, so
  `read_recovered`/`write_recovered`/`lock_recovered` were one-line
  pass-throughs with historical names; all ~190 call sites now call
  `.read()`/`.write()`/`.lock()` directly. Pure rename, no behavior change.
  `docs/kern/safety-architecture.md` deleted with the cruft purge below
  (stale `src/wire.rs` paths throughout); its ROADMAP reference now points at
  git history.
  Decided by: delete-superseded.

- 2026-07-20 — Cruft purge: ~4.5k lines of verified-dead code removed after a
  six-area audit (every deletion grep-verified to have zero non-test callers).
  Largest cuts: the legacy MCP client stack in trnsprt (`client.rs`,
  `registry.rs`, `inproc.rs`, `transport.rs`, `ServerId`) plus the `test-utils`
  and `logsink` crates that existed only to serve it; the test-only search RPC
  service (`SearchSvc`, mock, service DTOs); nine of thirteen `KernRpc` methods
  whose per-verb wire path was superseded by the generic `call_tool` dispatcher
  (only `health`/`shutdown`/`call_tool`/`list_tools` had live clients — the
  server keeps exactly those four); the orphaned `bench_support/backend.rs`
  A/B harness; `base/descriptors.rs` (a descriptor seed never wired — distill's
  claim kinds are a disjoint set); the `config-io` crate collapsed into
  `config::io` with unused `load`/`save` dropped. Dead config surface removed:
  `ServeConfig.{addr,core_addr,gossip,mcp_sse}`, `Config.log_level`, root CLI
  `--embed-url`/`--embed-model` — all parsed, never read. Unused deps pruned
  (`unicode-segmentation`, `unicode-width`, `windows-sys`). Orphaned bench
  scripts and stale `.splinter` caches for deleted sources removed.
  Bug fixed on sight: MCP `tool_degrade` had drifted from the CLI copy — it
  mutated scores in place with no lamport bump or `push_delta`, so degrades via
  MCP never gossiped to peers. It now calls the shared
  `graph_ops::degrade_entity_reasons`; a test pins the pending delta.
  Kept deliberately: `KernPreMass` decode path (reads pre-V3 shards), the
  `migrate` command (legacy-dir users), `locks.rs` naming shim (192 call
  sites — churn without behavior change), the gnn activation zoo (coherent set
  behind the enum). `docs/kern/safety-architecture.md` referenced the deleted
  `src/wire.rs` layout throughout and was deleted rather than rewritten; its
  threat-model content is recoverable from git history.
  Decided by: delete-superseded, verify-before-claiming (grep per deletion,
  full workspace tests green), fix-bugs-on-sight (degrade), name-the-tradeoff
  (locks shim and compat decoders kept over churn/data-loss risk).

- 2026-07-20 — Hub completes: auto-start, stop verb, and the phase-3 ordering
  call. `kern mcp` now auto-starts a detached machine hub when none answers
  (`[hub] auto_start = false` opts out; every failure falls through to the
  legacy direct-connect path, so nothing regresses when the hub can't come up).
  `kern hub stop` / `HubRpc::stop` ends a detached hub over RPC — nodes stay
  up; without it the only way to stop an auto-started hub was kill. Ordering
  decided for gossip-vs-hub: **federation senders and semantics (ROADMAP §5
  a-e) build per-node first; the gossip transport moves hub-side together with
  the TLS work, since both rewrite the same wire layer and moving a half-built
  transport twice pays the migration cost twice.** §5x phase 3 stays open but
  is no longer "blocked on a decision" — the decision is made and recorded.
  Auto-attach is pinned end-to-end in `tests/hub_supervisor.rs`: one MCP
  initialize from a hubless machine leaves a hub, a hub-owned node, and a
  working proxy.
  Decided by: name-the-tradeoff (auto-start default-on vs opt-in; migrate-once
  for the transport), avoided-question-first (the ordering question answered
  instead of deferred again), verify-before-claiming (auto-start test-pinned).

- 2026-07-20 — `GraphGnn::unload` refuses to unload when no store is bound.
  Unloading is residency, never forgetting: `get` reloads a kern through the
  store, so with `store: None` the kern left RAM with nothing to come back from
  — silent, total loss of every entity in it. The guard lives in `unload`
  itself rather than at its two call sites (`enforce_kern_cap` at
  `graph.rs:207`, the idle sweep at `tick/idle.rs:43`), because one guard in the
  shared function is smaller than a guard in every caller and cannot be missed
  by the next one. Found by the idle-eviction work, which had guarded its own
  path; that local guard is now redundant with the real fix. Regression test
  verified by reverting.
  Decided by: fix-the-root, fix-bugs-on-sight.

- 2026-07-20 — Idle kerns page out to the store on a time trigger.
  `KERN_IDLE_TIMEOUT`/`KERN_IDLE_SWEEP_EVERY` had existed as constants with no
  implementation. The question answered first was whether this duplicates
  existing bounding: it does not. `enforce_kern_cap` is called from exactly one
  place — `GraphGnn::register` — so it is coupled to *write traffic*, and
  `max_loaded_kerns` defaults to `KERN_CAP_DISABLED`, meaning kern-level
  unloading never happens at all in the default configuration. Stigmergy GC is
  entity-scoped and forgets; this is kern-scoped and only pages out. Idle
  eviction is the one mechanism that releases a kern sitting untouched under the
  cap, which is what "an idle daemon still maintains itself" requires. New
  `tick/idle.rs` selects victims under a read guard then takes the write guard
  per victim, never across the sweep; root is filtered twice. The
  compare_exchange cadence gate was about to become a third copy, so it is
  extracted as `claim_slot` and the stigmergy-GC and disk-consolidate copies
  collapsed onto it. Named for the next reader: `unload` deliberately leaves the
  kern's entities in `entity_kern`/`entity_idx` — that looks like a leak but
  keeps unloaded kerns searchable, and a hit resolves its kern and triggers the
  transparent reload; stripping them would be a recall regression.
  Decided by: builtin-before-built (reuse the existing unloaded/QUARANTINE
  path), name-the-tradeoff.

- 2026-07-20 — `move_entity` validates before it mutates, and has a surface.
  The function relocated an entity between kerns and was thoroughly tested, but
  nothing could call it — no MCP tool, no CLI. It also carried silent data loss:
  **three** mutations ran before the destination check — the entity removed from
  the source, incoming edges restamped toward the destination, outgoing edges
  deleted from `reasons`/`by_from`/`by_to` — and only then did a missing
  `to_kern_id` return early. A bad destination id destroyed the entity and every
  outgoing edge, and left surviving incoming edges pointing at a kern that does
  not exist. Restructured as validate-everything-then-mutate rather than
  rollback: only three things can fail and all three resolve up front through
  immutable borrows, after which `&mut GraphGnn` is exclusive and nothing can
  invalidate them. The signature moved from a silent `()` to
  `Result<(), MoveError>` so the failure cannot be swallowed at any call site.
  Exposed as the `move` MCP tool; the source kern is derived via `find_entity`
  rather than supplied, removing a class of bad input. Regression test verified
  by reverting.
  Decided by: fix-the-root, fix-bugs-on-sight.

- 2026-07-20 — The gossip Fetch RPC round-trips. The receive half existed; the
  sender half and the routing table never did. The real gap was
  `resolve_question_from_peer`: a peer answers a Question by *naming*
  `sphere.entity_id`, we stamped `r.to`/`r.to_net_id`, and the body was never
  obtained — a dangling cross-network reason. It now fires `spawn_fetch_entity`,
  and fetched bodies merge through `merge_remote_entity`, the same path
  `EntitySync` already uses, so commutativity/associativity/idempotence and the
  `conf_alpha`/`conf_beta`/`unlinked_count`/`statements` exclusions hold
  unchanged. A response whose `entity.id` differs from the requested id is
  rejected as a hijack. `put_thought`/`lookup_thought` — the third dead pair from
  the same feature — are now load-bearing: holders are recorded on entity-sync
  and on answer, and `fetch_thought` prefers the direct holder before falling
  back to routing.
  Decided by: fix-the-root (the dangling reason was the defect, not the unused
  function).

- 2026-07-20 — Dead code deleted after verification: the `SGD` optimizer whole
  (only `Adam` is ever constructed, at `gnn/propagate.rs:94`; deleting just
  `with_momentum` would have stranded an untested momentum branch),
  `bench_support/compare.rs` (a vector-backend comparison harness from the
  abandoned Qdrant-baseline effort — dead since it was written, and NOT
  superseded by the new paired-A/B eval work, which compares LoCoMo probe logs:
  different inputs, different metric, shared word), the `gnn` tensor and graph
  accessors, `gnn/backward.rs` normalization helpers superseded by
  `base::math::l2_normalize`, `tick/cluster.rs::largest_cohesive_cluster` (an
  unused alias of `best_cluster`), the legacy `trnsprt` MCP-client registry
  methods, and the `config-io` dir helpers duplicated inline in `config/mod.rs`.
  `COLD_COMPACT_MIN_BYTES` and `DEFAULT_WEIGHT_{CONTENT,REASON,EDGE}` were WIRED
  instead of deleted — the compaction guard is now live, and the weight literals
  existed inline in three places. `DEFAULT_WEIGHT_SCORE` is renamed
  `DEFAULT_WEIGHT_EDGE`: it feeds a field called `edge`, and the name mismatch
  was the mechanism by which it got orphaned.
  Three were kept after investigation contradicted the "unused" reading:
  `kern_rpc`'s methods are macro-generated by `service!` and consumed across a
  *process* boundary, so no in-repo caller could exist; GNN weights already
  persist through `Kern.gnn_weights` and the store, so `save_weights`/
  `load_weights` were a redundant second mechanism; and `MCP_VERSION` duplicated
  `trnsprt::PROTOCOL_VERSION` from a crate kern depends on, so wiring it would
  have been circular. `DEFAULT_DECAY` was deleted rather than wired because its
  value had diverged from the config it supposedly seeded (0.45 vs 0.25) —
  wiring it would have silently changed retrieval.
  Decided by: delete-superseded, verify-before-claiming.

- 2026-07-20 — Hub phase 2 + merge: the hub now manages node lifetime, not just
  spawn. Nodes report `HealthRes.idle_ms` — stamped at the MCP dispatch core
  *and* every typed `KernRpc` method (the typed surface bypasses `call_tool`,
  and an untracked RPC client would be unloaded mid-use), with health polls
  excluded so the hub's own probe can't keep a node warm. The reaper re-checks
  idleness under the per-root lock before unloading (a tool call can land
  between poll and kill), unloads only hub-owned nodes (a hand-started daemon
  is the user's to stop), never trusts `idle_ms == 0` (pre-field daemons), and
  polls at least as often as the threshold. `kern hub merge <src> <dst>` lands
  the cross-kern half of the original plan: both daemons stopped, offline CRDT
  union via `absorb_graph`, src never written. Found and fixed at the root
  while testing: `Config::load(root)` with no config file inherited serde's
  default `data_dir` — pinned to the *process* cwd — so any cross-root load
  read (and would have written) the caller's own store; configless loads now
  re-pin to the passed root, regression-tested. All lifecycle behavior is
  pinned in `tests/hub_supervisor.rs` against real processes.
  Decided by: fix-the-root (the config re-pin, not a merge-side workaround),
  name-the-tradeoff (owned-only unload; double-check window),
  verify-before-claiming (idle unload and merge proven end-to-end).

- 2026-07-20 — The answer prompt stops discarding retrieved evidence.
  `ANSWER_MAX_THOUGHTS = 5` capped the answer prompt at five facts while
  retrieval delivers up to `max_deliver_results = 25` — so the pipeline
  seeded, expanded, reranked and delivered ~24 claims per probe and then
  threw 19 away before the model saw them. Measured against real data,
  that ceiling buys nothing: distilled claims average **79 chars** (p50 75,
  max 227), so all 25 occupy ~493 tokens — **6% of the answerer's 8192
  context**. The cap discarded 80% of the evidence to save 6% of the
  window, and any gold-supporting claim ranked 6th-24th was found and then
  hidden. `answer_max_facts` is now a `RetrievalConfig` field (default
  unchanged at 5 pending the A/B) with validation: 0 is rejected (every
  answer would abstain) and exceeding `max_deliver_results` is rejected as
  dead config. `answer_prompt_from` now renders every fact it is handed —
  how many to include is retrieval policy, applied by the caller that holds
  the config. `locomo_eval --answer-facts N` exposes it. This is tuning of
  the full pipeline, not a disabled stage.
  Also: `--no-hyde`/`--no-rerank` are relabelled DIAGNOSTIC ONLY in CLI
  help and docs, because a fast number from a disabled stage measures
  something kern does not ship; speed must come from making the full
  pipeline cheaper. And eval reports now carry contention-immune LLM work
  counters (`llm_calls`, `llm_prompt_chars`, `llm_completion_chars`, per
  probe) — wall clock on this shared box produced two confident wrong
  conclusions today (that concurrency was hurting, and that HyDE was worth
  1.7×; HyDE in fact fires on 11.7% of probes because it is gated to
  queries under 6 tokens), so "does A do less work than B" is now
  answerable without a quiet machine.
  Decided by: verify-before-claiming (the 5-vs-25 ceiling was measured
  against claim lengths, not assumed), fix-the-root (the cap, not the
  symptom), name-the-tradeoff (more facts cost context but the budget is
  6%). Supersedes: the fixed five-fact answer prompt and wall-clock-only
  speed comparisons.

- 2026-07-20 — One binary, two roles: `kern hub` (machine-level control plane)
  supervises per-project node daemons instead of every project process fending
  for itself. The hub owns lifecycle only — `resolve(root)` spawns or adopts a
  node and returns its socket, `unload` drives the new `KernRpc::shutdown`
  (save-then-exit over RPC, no signals), a reaper drops dead entries; the data
  path stays client→node direct, so query latency is untouched. `kern mcp` asks
  the hub first and falls back to the legacy self-spawn path when no hub runs —
  rollout is opt-in by starting `kern hub`. Chosen over two separate binaries:
  same build means zero hub↔node version skew, and the "fast node" comes free
  since a node skips hub scaffolding. Chosen over hub-as-proxy: a proxy hop
  taxes every query to simplify connect-time only. Deferred deliberately:
  idle auto-unload (no honest activity signal yet — resolve time lies when MCP
  clients hold long connections), gossip hub-side (phase 3, ROADMAP §5x).
  Fixed on sight: a rebuild unlinks the running binary, `/proc/self/exe` reads
  "<path> (deleted)", and a long-lived hub could never spawn again — the marker
  is stripped since the fresh binary sits at the original path.
  Hardened to production in the same change: hub `canon()` applies
  `Config::resolve_root` after canonicalize (a booting node re-pins to the
  nearest `.kern` ancestor; without the same re-pin the hub ready-waits on a
  socket the node never binds and strands a live daemon), the resolve lock is
  per-root (one project's ~10s cold boot must not block another's connect),
  and `status` probes adopted nodes' sockets (a `child: None` entry has no
  `try_wait` and would report alive forever). Pinned by unit tests plus an
  end-to-end suite (`tests/hub_supervisor.rs`) that boots a real hub and node
  in an isolated `XDG_RUNTIME_DIR`.
  Decided by: name-the-tradeoff (single binary vs skew, control plane vs proxy),
  avoided-question-first (idle signal named and parked, not guessed),
  fix-bugs-on-sight (the deleted-exe spawn failure),
  verify-before-claiming (the lifecycle is test-pinned, not smoke-tested once).

- 2026-07-20 — Heat is decayed before the GC compares it, and the configured
  `[heat]` settings actually reach the deposit path. `is_cold_victim`
  (`tick/stigmergy.rs`) tested the raw stored `entity.heat` against
  `COLD_HEAT_THRESHOLD`; `heat::decayed` was correct but only ever reached
  through `heat::deposit`, so an entity that was hot once and never touched
  again kept its stale heat forever and evaded cold-tier eviction permanently —
  a direct breach of the "hot graph stays bounded" vision test. Separately,
  `pulse` hardcoded `HeatConfig::default()`, so a user's `half_life_secs` and
  `deposit_traversal` overrides were silently dropped. `TickContext` now carries
  `heat_cfg`; `pulse_with_heat` threads the real config from `cfg.heat`, the
  single source of truth. A pre-existing test that appeared to cover this
  (`heat_above_threshold_is_preserved_even_when_old`) set no `heat_updated_at`,
  so `decayed` short-circuited on `since: None` and it passed vacuously — now
  tightened into a real assertion. Named for the next reader: `pulse` still
  defaults the config at `gossip/handler.rs:197,213,268`, because
  `handler::Deps` carries no `Config` at all; behaviour there is unchanged from
  before, not regressed, and the fix is a one-line swap once that struct carries
  config.
  Decided by: fix-the-root (decay at the comparison, not at every call site),
  name-the-tradeoff (the handler seam is stated, not hidden).

- 2026-07-20 — One dedup decision, one threshold. Ingest deduped at a
  configurable `INGEST_DEDUP_THRESHOLD` (0.95) while `accept` hardcoded
  `DEFAULT_DEDUP_THRESHOLD` (0.92), so a claim scoring in the gap was judged new
  by ingest, fully built and embedded, then dropped by `commit_entity` with
  `deduped: true` — neither stored nor merged into the survivor, no
  `observe_support`, no `Rephrase` edge. Silent content loss reported to the
  caller as a successful dedup. `accept_with_dedup` now takes the configured
  value; `DEFAULT_DEDUP_THRESHOLD` is deleted. A second boundary bug found on
  sight: `is_duplicate` used `>` where `find_duplicate` uses `>=`, so a claim
  scoring exactly at the threshold was a duplicate to one check and not the
  other — both are `>=` now. Threading beats merging the constants because a
  user config above 0.95 would have re-opened the gap. Named for the next
  reader: the two checks still run *different queries* — `find_duplicate` hits
  `entity_idx` alone, `is_duplicate` also searches `gnn_entity_idx` and blends
  `0.4*content + 0.6*gnn` — so they can still disagree, and the residual fix is
  to make `commit_entity`'s duplicate branch merge like `update_existing_entity`
  instead of dropping.
  Decided by: fix-the-root, fix-bugs-on-sight (the `>`/`>=` split was found
  while fixing the threshold).

- 2026-07-20 — `src/wire.rs` deleted; the three surviving validators live in
  `base/validate.rs`. 451 of its 454 lines were DTOs with zero consumers,
  superseded by `trnsprt::kern_rpc::dto`. The validators stay in `base`, not
  `trnsprt`: the transport crate has no dependency on `kern` and these need
  `base::types::EntityKind`, so moving them there would invert the dependency —
  and they are domain validation, not wire framing, regardless. Renamed with the
  concept: `validate_wire_conf` → `validate_conf`, `WireError` → `ValidateError`.
  Decided by: delete-superseded.

- 2026-07-20 — The default local surfaces are authenticated. The kern_rpc Unix
  socket is `chmod 0600` after bind, and `serve_http` requires a bearer token
  (auto-minted at `<data_dir>/mcp-token`, created `0600` via
  `create_new` so the secret is never briefly world-readable) on both the POST
  and the SSE GET — the stream had been an open keepalive. Two premises were
  corrected by measurement rather than assumed: HTTP is opt-in (it binds only
  when `--mcp-addr` is passed), not exposed by default; and a bound socket at
  the default `umask 022` is `0755`, not `0777`, so Linux's write-bit check on
  `connect()` already blocked other users — the real exposure was
  umask-dependent. Named for the next reader: bind-then-chmod leaves a sub-ms
  window, chosen over a `umask` flip because umask is process-global and this
  daemon is multi-threaded, which would have raced every unrelated concurrent
  file creation; the race-free option (a private 0700 parent dir) would change
  the socket path and break rendezvous with running clients.
  Decided by: verify-before-claiming (both premises measured), name-the-tradeoff.

- 2026-07-20 — Statements no longer federate. `union_statements` in
  `base/merge.rs` merged them as a grow-only union, but entity ids *are*
  `content_hash(text)`, so honest replicas hold identical statements by
  construction and the union is provably a no-op — except when a peer asserts
  content its id does not hash to, which appended peer-controlled text into the
  lexical index and the digest. Statements join `conf_alpha`/`conf_beta`/
  `unlinked_count` on the never-import list; the senderless
  `CrdtTarget::Statements` arm becomes an explicit rejecting no-op so an older
  peer still cannot inject. No tombstones and no `OrSet`: removals are already
  encoded by the id changing, so a tombstone set would be permanent unbounded
  metadata solving a problem content-addressing had solved — and `statements` is
  positional (`ChunkPart.index` indexes into it), so an `OrSet<String>` would
  have silently broken chunk rendering. The four hand-rolled last-writer-wins
  comparisons are consolidated into `crdt::lww_wins`, pinned by a
  behaviour-preservation test. Named for the next reader: a genuinely divergent
  same-id entity now stays divergent instead of converging to a union — correct,
  because union of conflicting content under one hash is corruption, not
  convergence.
  Decided by: fix-the-root, name-the-tradeoff.

- 2026-07-20 — Dead code deleted after verification: `search_adaptive` +
  `AdaptiveEfConfig` + the never-read `adaptive_ef_*` config block,
  `ModeWeights.lexical` (defaulted 0.0 in all three modes and never read by
  `score_neighbor`; lexical retrieval is already wired correctly as a BM25
  channel fused by RRF, which is the right place for it — the field was a
  vestige of an abandoned linear-blend design), and the unused `PnCounter` /
  `LwwRegister` / `OrSet` types. `GCounter` stays; it is live.
  `refine_edges` was deleted, briefly restored on a mistaken premise, and
  deleted again — see the correction below.
  Decided by: delete-superseded.

- 2026-07-20 — Correction to the record. `refine_edges` was restored on the
  claim that it was the only producer of `CrdtTarget::ReasonScore` deltas,
  leaving that target receive-only. That claim was false and was relayed without
  verification: `degrade_entity_reasons` (`commands/graph_ops.rs:271`) is a live
  producer, so the CRDT half was never stranded. Two supporting claims were also
  false — `FEATURES.md` never described an "Edge refine" feature, and its CRDT
  section already stated correctly that no `LwwRegister`/`OrSet` type exists.
  On re-examination the function could not work regardless: nothing in
  production increments `traversal_count`, so its cadence gate reads a counter
  that is always 0 locally, and with gossip a peer shipping `tc == 10` would
  fire it on *every* query touching that edge, unbounded — it also needs a write
  guard and an LLM round-trip, so it can never run on the read path at any
  cadence. Re-deleted.
  Decided by: verify-before-claiming (the failure this entry records),
  delete-superseded.

- 2026-07-20 — `broadcast_pulse` reaches the MCP `pulse` tool. It was reported
  as dead code; it is an initialization-order bug. The `Server` was constructed
  at `commands.rs:631` with `None` while `start_gossip` does not return the real
  broadcaster until line 638, and the struct captures it by value — so the
  maintenance tick got a working broadcaster and the MCP tool silently never
  broadcast to peers. Server construction moved below `start_gossip`; nothing in
  between depended on it. The same tool now also uses the configured heat
  settings via `self.cfg.heat`, which `Server` already carried. Named for the
  next reader: this fix is verified by compile and inspection, not by a test —
  covering it needs a booted daemon with gossip enabled, which the harness does
  not do.
  Decided by: fix-bugs-on-sight, verify-before-claiming (the coverage gap is
  stated rather than implied).

- 2026-07-20 — `capture` is gone; the intake is the only name, and it now
  reaches the disk and the config file the 2026-07-17 rename deliberately
  stopped short of. `CaptureConfig` → `IntakeConfig`, `[capture]` → `[intake]`,
  `.kern/capture/` → `.kern/intake/`, `spawn_capture` → `spawn_intake`,
  tracing target `kern.capture` → `kern.intake`, and the docs site's
  `howto/capture-recall.md` → `howto/intake-recall.md` with the nav entry to
  match. Cause: that rename kept the old name on disk to avoid a migration, and
  the half-rename cost more than the migration would have — a reader hit
  `intake` in the code, `capture` in `kern.toml`, and had no way to tell whether
  they were the same thing. Worse, this session invented a *third* word for it
  ("inbox") in comments and roadmap prose before anyone noticed, which is what a
  vocabulary with two live names invites. All three now read `intake`.
  The migration the old decision feared is nine lines: `migrate_legacy_dir`
  renames `.kern/capture` to `.kern/intake` on daemon start, and only when the
  new path is free — an existing intake dir means the move already happened, and
  merging a re-created legacy dir over live state is never correct. Both
  branches are tested. No serde alias or compat shim is kept: `Config` is
  `#[serde(default)]` with no `deny_unknown_fields`, so a stale `[capture]`
  section is ignored rather than fatal, and all four real `kern.toml` files on
  this machine carry the section empty — header and comment, zero settings — so
  nothing tunable is dropped. Named for the next reader: an operator who *had*
  tuned `[capture]` would lose those values silently on upgrade; accepted
  because no such config exists, and rejected as a permanent alias because one
  concept with two accepted spellings is the defect being fixed.
  The digest knobs stay inside `[intake]` rather than splitting into their own
  section — one configuration, per the call made here.
  Decided by: delete-superseded (one name, no alias), fix-the-root (rename the
  disk and the config, not just the code), name-the-tradeoff (the silent
  config-value loss). Supersedes: the 2026-07-17 decision to leave the on-disk
  layout and config section named `capture`.

- 2026-07-20 — The intake accepts what is dropped in it,
  instead of silently eating everything it did not recognise.
  `drain_entry` gated on `extension == "txt"` and returned early otherwise —
  no log, no error, no move to `done/` — so a `.md`, a `.json`, or an
  extensionless note sat in the intake forever *looking accepted*. Silent
  loss on the exact gesture the intake exists for. The extension allowlist is
  replaced by asking what the file is: anything that reads as UTF-8 gets in,
  `.txt` stays the session-transcript lane and is distilled into claims, and
  everything else is a document stored whole through the same path the file
  watcher uses (`Source::File`, `EntityKind::Document`). Binary — an
  `InvalidData` read — is quarantined into `failed/` with a warning rather
  than retried forever; a genuine IO error is still left in place, because
  those are transient and quarantining them would lose data. Empty files
  archive straight to `done/`. Consequence worth naming: **documents need no
  reason LLM**, only the embedder, so `spawn_capture` now always starts the
  drain and downgrades the missing-reason-model case from "intake dead" to a
  warning that transcripts specifically will wait. `intake::run` takes
  `Option<LlmFunc>` to carry that. Two behaviours the design deliberately
  keeps apart: distillation is what a *transcript* gets, not what everything
  gets — a large document routed through the one-shot distill prompt would
  truncate at the model's context window, while the document path chunks.
  Ceiling marked in place: a file still being copied can read as
  valid-but-truncated text; an mtime-settle check is the upgrade path if
  partial drops appear. New test asserts the whole promise in one run — a
  `.md` document ingests with `None` for the LLM, and a planted PNG lands in
  `failed/`. 826 workspace tests green.
  Decided by: fix-bugs-on-sight (silent data loss found while documenting the
  path), fix-the-root (the allowlist was the defect, not the missing
  extensions), name-the-tradeoff (transcript-vs-document routing, and the
  partial-write ceiling). Supersedes: the `.txt`-only intake filter and the
  reason-LLM gate on the whole drain.

- 2026-07-20 — The docs site moves from MkDocs Material to the Terminal
  theme (`mkdocs-terminal`, `gruvbox_dark` palette), and the book is filled
  out to 16 pages across Concepts and How-to. Cause: the theme was chosen
  for look; the monospace terminal aesthetic matches what kern is. Tradeoff,
  named, and it is not free — Terminal ships neither Mermaid nor admonition
  styling, both of which Material gave for nothing and both of which the
  written pages depend on. Rather than drop the content,
  `docs/site/assets/extra.css` styles `.admonition` (danger/warning carry
  their own colour, so the unauthenticated-federation notice keeps its
  visual weight) and `docs/site/assets/mermaid-init.js` bootstraps Mermaid
  11 from jsDelivr. That init file injects its own `<script type="module">`
  because Terminal's `base.html:34` renders `extra_javascript` as a plain
  `<script src>` and drops the `type`, which silently breaks a bare ESM
  import — recorded because the failure is invisible at build time and
  `--strict` stays green. Social preview cards were requested but Material's
  `social` plugin cannot run under another theme; `docs/overrides/main.html`
  hooks Terminal's `extrahead` block to emit per-page OpenGraph/Twitter
  title, description and canonical URL instead. Link unfurls therefore carry
  the right text but no generated image, and the CI image pipeline
  (cairo/pango) is not needed after all. Supersedes the Material theme
  configuration recorded earlier today.
  Decided by: name-the-tradeoff, verify-before-claiming.

- 2026-07-20 — The intake naming ban stops being remembered and starts being
  enforced: a `vocab` job in `ci.yml` fails any commit reintroducing the
  print-queue-style working name the 2026-07-17 rename scrubbed. Cause: that
  decision said "no alias or historical mention kept anywhere", and the word
  had already drifted back into three prose files (`CHANGELOG.md` line 504,
  `.splinter/src/config/mod.splinter.md`, `.splinter/src/config/wsl.splinter.md`)
  — all written *after* the scrub. A hand-run scrub cannot hold a vocabulary;
  a failing check can. All three sites now say the intake retries the job.
  Verified in both directions: the guard passes on the clean tree and fires on
  a reintroduced occurrence. Also fixed while here: `ROADMAP.md` §7e cited an
  outage queue at `ingest/queue.rs` — that file does not exist and never did;
  the retry behaviour it described is the intake's (`ingest/intake.rs`,
  `finalize` archives only on full success). The citation was inherited
  unchecked from the Alois plan when §7 was folded in.
  Decided by: fix-the-root (enforce the ban instead of re-scrubbing),
  verify-before-claiming (a cited path that does not exist is a false claim).
  Supersedes: the hand-run scrub as the ban's only enforcement.

- 2026-07-20 — The Alois integration plan folds into `ROADMAP.md` §7 as the
  embeddable-endpoint track, and `docs/ALOIS-INTEGRATION-PLAN.md` is deleted.
  Cause: the work it described was never Alois-specific — ACL plus a request
  principal, a review/draft lifecycle, source-trust weighting, and
  `forget_by_source` retention are what *any* host system needs to mount kern
  as its reasoning store instead of Zep or a vector DB. Filed under one
  consumer's name it read as a side integration; it is the second-most
  valuable track after the eval gap, because it converts kern from one agent's
  memory into a memory layer other agentic workflows embed. Ordering is
  preserved from the audit: ACL gates everything, review builds on its
  `QueryOptions` work, source-trust runs parallel. Three constraints carried
  over verbatim because they are easy to lose: ACL is caller-asserted and
  trust ends at the process edge; Facts are GC-immune but never ACL-immune;
  and `forget_by_source` is the sole sanctioned bypass of the Fact guard, so
  it must be explicit and never default. In-kern token metering stays
  deferred — gateway-side metering needs zero kern change. Decided by:
  delete-superseded. Supersedes: `docs/ALOIS-INTEGRATION-PLAN.md`.

- 2026-07-20 — `ROADMAP.md` becomes the single source of truth for state and
  open work, and eight planning documents that duplicated it are deleted:
  `docs/aspiration.md`, `docs/vision.md`, `docs/landscape.md`, `docs/v2.md`,
  `docs/federation-roadmap.md`, `docs/federation-integration-plan.md`,
  `docs/oracle/FEATURE-AUDIT.md`, `docs/kern/board-unblock-plan.md`,
  `docs/kern/locomo-improvements.md`. Cause: nine files each held a partial,
  separately-dated view of what was left, and they disagreed — the feature
  audit claimed the hook layer both shipped and was retired, the federation
  plan said the Delta sender did not exist while `start_delta_flush` had
  been live since 2026-07-17, and `docs/landscape.md` still said no LoCoMo
  baseline existed a day after one was recorded. Every open item was
  re-verified against source at HEAD before being folded in, and the
  contradictions resolved in favour of the code: federation Phase 1 landed
  as inline lamport-stamped LWW fields plus `union_statements`, not as named
  `OrSet`/`LwwRegister` types, so `crdt.rs` is correctly still `GCounter`
  only; Pulse/Question senders and `AntiEntropy` are genuinely absent. The
  new file carries the north star and recorded baseline, the supersession
  argument against Zep/Mem0/Letta/Qdrant, the eval sequence, retrieval,
  federation, safety, non-goals, and the repo laws — including a fourth law:
  new work goes in this file, never a new document. `docs/kern/` is now
  reference and measurement records only. Decided by: delete-superseded,
  with verify-before-claiming governing every folded status marker.
  Supersedes: the nine deleted documents and the previous nine-question
  `ROADMAP.md`.

- 2026-07-20 — Documentation moves from mdBook to MkDocs Material, and the
  site publishes itself. `docs/book/` is gone; the three real pages live in
  `docs/site/` (`introduction.md` became `index.md`), configured by a
  root `mkdocs.yml`. Cause: `just docs` was dead — it invoked a `doc-gen`
  crate in a sibling `../shared` workspace that does not exist on any
  checkout, and `SUMMARY.md` was never generated, so `mdbook build` could
  not succeed for anyone. Rather than resurrect a generator nothing depends
  on, the pages are hand-written and the generation step is deleted.
  MkDocs Material subsumes the whole `book.toml` surface in stock
  configuration — search, dark/light palette, edit-url, and Mermaid via
  `pymdownx.superfences` — so the vendored `mermaid.min.js`,
  `mermaid-init.js`, `theme/custom.css`, `theme/custom.js`, and
  `flows.toml` (an empty hand-seeded flow list feeding the dead generator)
  are all deleted rather than ported. No plugin beyond stock `search` is
  installed: the MkDocs catalog is a directory to consult when a need
  appears, not a set to adopt up front. `.github/workflows/docs.yml`
  builds `--strict` on every docs-touching PR and runs `mkdocs gh-deploy`
  on master; `docs/requirements.txt` pins `mkdocs<2` because Material's
  maintainers have flagged MkDocs 2.0 as removing the plugin system with
  no migration path. `.pi/update.sh` is created so a fresh checkout
  installs the docs toolchain via `/doctor`. Supersedes the mdBook
  toolchain and the `docs`/`docs-watch`/`docs-serve`/`docs-check` recipes,
  replaced by `docs`/`docs-serve`/`docs-deploy`.
  Decided by: delete-superseded, builtin-before-built, fix-the-root.

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
  so every embed returned a transient connect error and the intake retried the
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
