# Aspiration — State-of-the-Art Agent Memory, Measured

**North star:** kern equals or beats Zep/Mem0-class agent-memory systems on
LoCoMo / LongMemEval-style evals while staying local-first, self-contained,
in-process, per-cwd — no cloud, no query-time LLM required, offline-capable.

The old north star ("Supersede Qdrant in Every Regard", kept as Appendix A)
aimed kern at a general-purpose vector-DB fight it doesn't need to win and
can't win solo. kern's actual competitive set is agent memory — Zep/Graphiti,
Mem0, Letta — and against those kern is architecturally credible today:

| kern property | Zep/Graphiti | Mem0 | Letta |
|---|---|---|---|
| Per-project self-maintaining graph (per-cwd) | ❌ hosted/service | ❌ | ❌ |
| No query-time LLM on the default path (sub-ms graph recall) | ❌ | ❌ | ❌ |
| Local-first, offline, single binary, no network hop | ❌ | ❌ | partial |
| Self-forgetting (stigmergy / decay / evict / cycle-safe GC) | ❌ | partial | ❌ |
| Graph + dense ANN + lexical + GNN re-embedding in one process | partial | ❌ | ❌ |

## Claim standard (the only scoreboard that counts)

- **Measure, don't assume.** No SOTA claim without a run.
- Published leaderboard gaps **under ~10 points are noise**: 2026 audits found
  LoCoMo's answer key ~6% wrong and LLM-judge leniency inflating scores. So
  kern's bar is **multi-seed runs with error bars and a strict judge**, not a
  single-seed headline beating someone's blog number.
- Harness exists: `src/bin/locomo_eval.rs` (feature `bench`) — F1 / ROUGE-L /
  LLM-judge per category, adversarial abstention, context size, query latency,
  `--json` for CI diffing.

---

## Plan (ordered by leverage)

### Tier 0 — baseline first (unblocks everything)
- [ ] **Run `locomo_eval` end-to-end** with the default local models
      (`cargo run --features bench --bin locomo_eval`) and **record the
      baseline** — per-category scores + latency, multi-seed, committed as the
      reference JSON. Every later change is judged against this number.
- [ ] Add the strict-judge / multi-seed loop (seeds + error bars in the report).
- [x] Profile query latency — `kern profile` (`profile_cmd`): graph is sub-ms;
      the LLM path (12–21s/call) is the delay villain.

### Tier 1 — retrieval quality (moves the eval score)
- [ ] Gate HyDE off on strong lexical/cache hits (skip query expansion when the
      cheap path already wins).
- [ ] Move the dense seed merge `merge_hits` (raw `0.4c+0.6g` blend) onto RRF —
      fragile across scales; `fuse::rrf` already fuses the answer-layer lists.
- [x] Filtered ANN end-to-end — all three seed sources filter during retrieval
      on `QueryOptions::is_active`; recall@10 A/B validated (`9386de0`).
- [x] RRF hybrid fusion at the answer layer — `fuse::rrf`, `cfg.rrf_k`,
      `SweepParam::RrfK`.
- [x] Workload regression trace + tuning loop — `traces/workload.json` (200
      docs / 50 queries, deterministic), `just bench-workload`; first sweep
      found `mmr_lambda` 0.45 over-diversifying (recall@10 0.925 → 1.0 at
      0.75). Baselines in `docs/kern/bench-retrieval.md`.
- [ ] Temporal-reasoning and multi-session categories are where LoCoMo-class
      evals punish memory systems — tune graph expansion + distill against the
      per-category baseline, not intuition.

### Tier 2 — latency (Stage A carries over)
- [x] A1 Default sub-ms graph path — `answer:false` never touches the LLM
      (`answer_llm_args`, regression test `answer_false_passes_no_llm_or_embedder`).
- [x] A2 Semantic query cache — cosine≥0.97 + version-stamp invalidation.
- [ ] A3 Cut the LLM call when it IS needed — streaming, capped `num_ctx`,
      warm-keeping shipped (`llm.rs`); *remaining:* HyDE gating (Tier 1) +
      speculative decode (qwen3.5:0.8b draft → 4b generator).
- [x] A4 Lock-scoped answer path — no write guard held across an LLM await.

### Tier 3 — durability as memory-safety (not DB parity)
A memory that loses what it was told is not a memory. This tier exists because
agents trust kern with state, not because Qdrant has these boxes checked.
- [ ] **Snapshots / restore** — reuse `persist::save_all` under a read lock into
      a timestamped dir + `manifest.json`; restore = validate + load_dir +
      atomic swap. No second persistence path.
- [ ] **WAL** — append-only op-log + replay-on-start over the bincode shards
      for ordered crash recovery.
- [ ] Memmap tiering + background compaction (`diskann.rs` memmap +
      `cold.rs`/`heat.rs` tiers exist) so startup and RAM scale sub-linearly.

### Hold the line (don't regress)
- [ ] DiskANN recall@k + latency edge; int8 recall parity; traversal-time +
      graph-level filtering; graph/GNN/self-org/cache moat; per-cwd isolation.

## Non-goals — unless kern becomes a database product

Demoted from the old roadmap. None of these move an agent-memory eval score;
all of them are table stakes only in a multi-tenant hosted-DB business kern is
not in. Revisit only if that business materializes.

- Distributed sharding (Raft-coordinated placement)
- Replication + write-consistency factor
- API key / JWT-RBAC / TLS / audit logging
- Public REST + gRPC APIs + multi-language SDKs
- Multitenancy (tenant payload index)
- GPU-accelerated index building
- SPLADE sparse vectors / ColBERT multi-vector / product quantization —
  re-promote any of these individually **iff** a Tier-0 baseline shows a
  retrieval-quality gap they would close.

## Repo laws (unchanged by the re-aim)

1. **Append-only bincode** — persisted enums/structs grow by appending only;
   guard schema touches with a round-trip test.
2. **No pluggable/fallback backend** — kern is all-internal, in-process,
   self-contained. Mounting someone else's store forfeits the structural
   advantage (no network hop, GNN vectors coupled in-memory).
3. **One dispatch core** — every surface (MCP, HTTP, any future one) goes
   through the single `tools::dispatch`; never a second copy.

---

# Appendix A — Historical framing: "Supersede Qdrant in Every Regard"
*(vector-DB parity reference, superseded 2026-07-16 by the agent-memory north
star above; preserved for history — the ✅ rows are still real capabilities to
hold, the ❌ rows are now Non-goals or demoted tiers)*

**Old north star:** every category below reads ✅. kern equals or beats Qdrant
on its own turf (vector DB) *and* keeps the layers Qdrant will never have
(graph memory, GNN, self-organization, LLM answers) — all in one
self-contained, in-process, per-cwd binary with no network hop.

Comparison baseline: real `qdrant/qdrant` repo + docs, v1.13+ feature line.
Test surface: **562 test fns defined** across 114 files (workspace-wide,
`#[test]`/`#[tokio::test]`). The earlier "441 passed / 0 failed (6 suites, green)"
headline predates recent additions — re-run `cargo test` to refresh the pass count
before quoting it (measure, don't assume).
*[Stale — 2026-07-16]* These counts are historical snapshots. The Criterion
`benches/` dir and its `[[bench]]` targets were removed in `1465a5e`; the
`retrieval_bench` binary (`src/bin/retrieval_bench.rs`) remains the live bench
entry point. Re-count before quoting.

## Where kern already leads (hold the line) — 8/27 ✅

| Category | Status | Aspiration |
|---|---|---|
| Dense vector ANN (HNSW + DiskANN) | ✅ | Stay ahead: keep DiskANN edge, beat Qdrant recall@k *and* latency. |
| int8 / scalar quantization | ✅ | Maintain recall-validated parity. |
| Filtered ANN (filter during traversal) | ✅ | Keep traversal-time + graph-level filtering. |
| Graph / relational memory | ✅ | Widen the moat — Qdrant has no equivalent. |
| GNN re-embedder | ✅ | Widen the moat — Qdrant has no equivalent. |
| Self-organization (stigmergy / spawn / evict / cycle-safe GC) | ✅ | Widen the moat. |
| LLM answer synthesis (HyDE / rerank / answer) | ✅ | Cut latency from 12–21s → interactive. |
| Semantic query cache | ✅ | Keep cosine≥0.97 + version-stamp invalidation. |

## Where Qdrant still wins (the old climb) — 18/27 ❌ + 1 🟡

### Quantization depth
| Category | Now | Old target |
|---|---|---|
| Product quantization (up to 64×) | ❌ | ✅ ship PQ |
| Binary quant / TurboQuant (1-bit) | ❌ | ✅ ship binary + rescoring |

### Filtering & payload
| Category | Now | Old target |
|---|---|---|
| Payload / typed field indexing (keyword/int/float/bool/geo/datetime/text/tenant) | ❌ | ✅ |
| Full-text / geo / range / nested filters + cardinality planning | ❌ | ✅ |

### Vector models
| Category | Now | Old target |
|---|---|---|
| Sparse vectors (SPLADE) | ❌ | ✅ |
| Named / multi-vector per point (ColBERT late-interaction) | ❌ | ✅ |
| Hybrid query / RRF / structured prefetch | 🟡 RRF + multi-list hybrid live (`fuse::rrf`, `cfg.rrf_k` in `answer.rs`); structured-prefetch API ❌ | ✅ |
| Recommend / Discover / Context / distance-matrix APIs | ❌ | ✅ |

### Distribution & durability
| Category | Now | Old target |
|---|---|---|
| Distributed sharding (Raft-coordinated) | ❌ (gossip only) | ✅ — now a Non-goal |
| Replication + write-consistency factor | ❌ | ✅ — now a Non-goal |
| Snapshots / backup / restore | ❌ | ✅ — kept, as Tier 3 memory-safety |
| WAL / ordered crash recovery + per-point versioning | ❌ | ✅ — kept, as Tier 3 memory-safety |
| On-disk / memmap tiering + segment optimizer | ❌ | ✅ — kept, as Tier 3 |
| GPU-accelerated index building | ❌ (GPU = LLM only) | ✅ — now a Non-goal |

### Interface, security, ops
| Category | Now | Old target |
|---|---|---|
| REST + gRPC API + multi-language SDKs | ❌ (MCP + `trnsprt::http` axum serve + `kern_rpc`; no *public* REST/gRPC/SDK) | ✅ — now a Non-goal |
| API key / JWT-RBAC / TLS / audit logging | ❌ | ✅ — now a Non-goal |
| Multitenancy (tenant payload index) | ❌ | ✅ — now a Non-goal |
| Production-scale maturity / proven at scale | ❌ | ✅ |
| Head-to-head benchmark harness vs Qdrant | ❌ | ✅ build first — measure, don't assume |

### Old scoreboard
- **Then: 8 ✅ / 1 🟡 / 18 ❌** (vs real Qdrant v1.13+).
- **Old aspiration: 27 ✅ / 0 ❌** — retired; superseded by the eval-driven
  plan above.

### Strategic constraint (still decided, still true)
Mounting Qdrant as a backend yields a *superset*, **not** supersession — it
forfeits kern's only structural advantage (in-process, no network hop, GNN
vectors coupled in-memory) and makes kern strictly slower than raw Qdrant on
vector ops. Repo law forbids a pluggable/fallback backend, so anything kern
builds is all-internal. This constraint survives the re-aim; only the target
list changed.

**Repo-law flags carried by the old roadmap (still binding where the work
survives):** (1) binary-quant work would touch bincode-positional
`QuantizationMode`/`QuantizedVec` — append-only, guard with a round-trip test;
(2) any REST surface must not duplicate MCP dispatch (one `tools::dispatch`
core); (3) all stages in-process/self-contained — no pluggable backend
(no-compat law).

## Verification provenance

Status markers above were checked against the **working tree atop `9683c5c`**
(2026-06-10; ~76 modified tracked files uncommitted). Re-stamp when re-verifying.

| Claim | Verified against (symbol) |
|---|---|
| RRF / hybrid fusion (🟡) | `fuse::rrf` (`retrieval/fuse.rs`), live at `answer.rs`; `cfg.rrf_k`, `SweepParam::RrfK` |
| A1 `answer:false` sub-ms path (DONE) | `answer_llm_args` + test `answer_false_passes_no_llm_or_embedder` (`tools_query.rs`) |
| A3 latency bundle (PARTIAL) | `complete_stream`/`params.stream`, `*_NUM_CTX`, `*_KEEP_ALIVE`, `num_gpu:0` (`llm.rs`) |
| Tier-0 profiling (DONE) | `profile_cmd` / `kern profile` |
| `search_all_filtered` WIRED (dense+importance+lexical) | `seed.rs` filters all three seed sources on `is_active`; e2e recall@10 A/B `9386de0` (was: unwired atop `9683c5c`) |
| Snapshots / WAL / restore = ❌ | no backup/restore/WAL fns in `persist.rs` |
| Recommend/Discover/distance-matrix = ❌ | no such fns repo-wide |
| REST surface qualifier | `trnsprt::http` axum serve + `kern_rpc`; no public REST/gRPC/SDK |
| Test surface | **799 lib + 1 integration tests PASS** (verified `cargo test` 2026-06-12 atop `84ba856`); ~962 `#[test]`/`#[tokio::test]` attrs across 170 src files. *[Stale — 2026-07-16: predates the `benches/` removal in `1465a5e` and later changes; re-run before quoting.]* |

*Unverified (needs a run, not a grep):* actual recall@k / p50-p99 latency vs
Qdrant (the old Tier-0 harness, never built — and no longer a goal). *(The live
`cargo test` pass count row above is a verified historical snapshot.)*
