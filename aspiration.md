# Aspiration — Supersede Qdrant in Every Regard

**North star:** every category below reads ✅. kern equals or beats Qdrant on
its own turf (vector DB) *and* keeps the layers Qdrant will never have (graph
memory, GNN, self-organization, LLM answers) — all in one self-contained,
in-process, per-cwd binary with no network hop.

Comparison baseline: real `qdrant/qdrant` repo + docs, v1.13+ feature line.
Test surface: **562 test fns defined** across 114 files (workspace-wide,
`#[test]`/`#[tokio::test]`). The earlier "441 passed / 0 failed (6 suites, green)"
headline predates recent additions — re-run `cargo test` to refresh the pass count
before quoting it (measure, don't assume).

---

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

## Where Qdrant still wins (the climb) — 18/27 ❌ + 1 🟡 → target ✅

### Quantization depth
| Category | Now | Target |
|---|---|---|
| Product quantization (up to 64×) | ❌ | ✅ ship PQ |
| Binary quant / TurboQuant (1-bit) | ❌ | ✅ ship binary + rescoring |

### Filtering & payload
| Category | Now | Target |
|---|---|---|
| Payload / typed field indexing (keyword/int/float/bool/geo/datetime/text/tenant) | ❌ | ✅ |
| Full-text / geo / range / nested filters + cardinality planning | ❌ | ✅ |

### Vector models
| Category | Now | Target |
|---|---|---|
| Sparse vectors (SPLADE) | ❌ | ✅ |
| Named / multi-vector per point (ColBERT late-interaction) | ❌ | ✅ |
| Hybrid query / RRF / structured prefetch | 🟡 RRF + multi-list hybrid live (`fuse::rrf`, `cfg.rrf_k` in `answer.rs`); structured-prefetch API ❌ | ✅ |
| Recommend / Discover / Context / distance-matrix APIs | ❌ | ✅ |

### Distribution & durability (the production-DB tier — biggest gap)
| Category | Now | Target |
|---|---|---|
| Distributed sharding (Raft-coordinated) | ❌ (gossip only) | ✅ |
| Replication + write-consistency factor | ❌ | ✅ |
| Snapshots / backup / restore | ❌ | ✅ |
| WAL / ordered crash recovery + per-point versioning | ❌ | ✅ |
| On-disk / memmap tiering + segment optimizer | ❌ | ✅ |
| GPU-accelerated index building | ❌ (GPU = LLM only) | ✅ |

### Interface, security, ops
| Category | Now | Target |
|---|---|---|
| REST + gRPC API + multi-language SDKs | ❌ (MCP + viewer-internal axum HTTP + `kern_rpc`; no *public* REST/gRPC/SDK) | ✅ |
| API key / JWT-RBAC / TLS / audit logging | ❌ | ✅ |
| Multitenancy (tenant payload index) | ❌ | ✅ |
| Production-scale maturity / proven at scale | ❌ | ✅ |
| Head-to-head benchmark harness vs Qdrant | ❌ | ✅ build first — measure, don't assume |

---

## Scoreboard

- **Now: 8 ✅ / 1 🟡 / 18 ❌** (vs real Qdrant v1.13+).
- **Aspiration: 27 ✅ / 0 ❌.**

## Strategic constraint (decided)

Mounting Qdrant as a backend yields a *superset*, **not** supersession — it
forfeits kern's only structural advantage (in-process, no network hop, GNN
vectors coupled in-memory) and makes kern strictly slower than raw Qdrant on
vector ops. "Supersede in every regard" therefore requires building the missing
DB tier **inside** kern. Repo law forbids a pluggable/fallback backend, so the
path is all-internal.

**Next concrete move:** build the benchmark harness (measure the real recall@k +
latency gap), then close the production-DB tier — sharding, replication, WAL,
snapshots — since that is the largest, most-blocking cluster of ❌.

---

## TODO — path to 27/27 ✅ (ordered by leverage)

### Tier 0 — measure first (unblocks everything)
- [ ] **Benchmark harness vs Qdrant** — recall@k + p50/p95/p99 latency + RPS + RAM on a shared dataset (start from `retrieval_bench`). Measure, don't assume.
- [ ] Wire `search_all_filtered` into the live query path — filtered ANN end-to-end, kill post-filtering.
- [x] Profile query latency — `kern profile` (`profile_cmd`) breaks the timeline into graph (sub-ms) vs LLM hyde/answer/distill (~12–16s each). Confirmed: the LLM path is the delay, not the index.

### Tier 1 — production-DB tier (largest ❌ cluster)
- [ ] **WAL** — ordered crash recovery + per-point versioning over bincode shards.
- [ ] **Snapshots** — backup / restore (consistent point-in-time of shards + graph).
- [ ] **Replication** — write-consistency factor on top of gossip.
- [ ] **Distributed sharding** — Raft-coordinated placement (gossip → consensus).
- [ ] **On-disk / memmap tiering** + segment optimizer (background compaction).

### Tier 2 — interface, security, ops
- [ ] **REST + gRPC API** + multi-language SDKs — no *public* REST/gRPC/SDK yet (viewer-internal axum HTTP + `kern_rpc` already exist; see D1). Build over the shared `tools::dispatch`.
- [ ] **Auth**: API key / JWT-RBAC / TLS / audit logging.
- [ ] **Multitenancy** via tenant payload index.

### Tier 3 — vector-model & quantization depth
- [ ] **Product quantization** (up to 64×).
- [ ] **Binary quant / TurboQuant** (1-bit) + rescoring.
- [ ] **Sparse vectors** (SPLADE).
- [ ] **Named / multi-vector per point** (ColBERT late-interaction).
- [x] **RRF / multi-list hybrid fusion** — `fuse::rrf` live in `answer.rs` (`cfg.rrf_k`, `SweepParam::RrfK`). *Remaining:* **structured prefetch** API + RRF for the dense seed merge (`merge_hits` still `0.4c+0.6g`).
- [ ] **Recommend / Discover / Context / distance-matrix** APIs.

### Tier 4 — filtering & payload
- [ ] **Typed payload field indexing** (keyword/int/float/bool/geo/datetime/text/tenant).
- [ ] **Full-text / geo / range / nested filters** + cardinality-based query planning.

### Tier 5 — index build & proof
- [ ] **GPU-accelerated index building** (today GPU = LLM only).
- [ ] **Production-scale maturity** — soak tests, large-corpus validation, the proof.
- [ ] Cut LLM answer path 12–21s → interactive — streaming + `num_ctx` cap + `keep_alive` warm-keeping shipped (`llm.rs`); HyDE-gating on strong lexical hits + speculative-decode remain.

### Hold the line (8 ✅ — don't regress)
- [ ] Keep DiskANN recall@k + latency edge; maintain int8 recall parity; keep
      traversal-time + graph-level filtering; widen graph/GNN/self-org/cache moat.

---

## Step-by-step → robust, fluent memory engine with almost no delay

Grounded in a codebase feasibility scan (what already exists, so each step is
*extension*, not greenfield). **The delay villain is the LLM path (12–21s/call),
not the graph (sub-ms).** So latency work front-loads; parity work follows.

### Stage A — Near-zero perceived delay (latency first)
A1. **Default to the sub-ms graph path (DONE).** `answer:false` returns ranked
    graph hits with no LLM call — `answer_llm_args` (`tools_query.rs`) gates
    HyDE/rerank/synthesis off unless `answer:true` (regression test
    `answer_false_passes_no_llm_or_embedder`). Profiler confirms the graph engine
    is sub-ms — most recalls never touch the LLM.
A2. **Semantic query cache (DONE)** — cosine≥0.97 + version-stamp invalidation
    skips the ~33s LLM path on repeat/similar recalls. Widen hit rate; pre-warm.
A3. **Cut the LLM call when it IS needed (PARTIAL).** Shipped in `llm.rs`: token
    streaming (`complete_stream`, `params.stream`), capped `num_ctx`
    (`ANSWER/EMBED/REASON_NUM_CTX`), and Ollama warm-keeping (`*_KEEP_ALIVE` +
    ~4-min warm ping); reason runs CPU-only (`num_gpu:0`). *Remaining:* gate HyDE
    (skip query expansion on a strong lexical/cache hit) and speculative-decode
    (qwen3.5:0.8b draft → 4b generator).
A4. **Lock-scoped answer path (DONE)** — never hold a write guard across the LLM
    await; keep read/write guards minimal so concurrent recalls don't serialize.

### Stage B — Faster + lighter index (the quick-win bundle)
B1. **Binary quantization** — append `Binary` to `QuantizationMode` + `b:Vec<u8>`
    to `QuantizedVec` (bincode-APPEND only). 1-bit hamming for candidate gen, then
    rescore with the retained f64 `HnswNode.vec` (kept even when quantized). 64×
    smaller, faster traversal. Validate recall@k like int8.
B2. **RRF hybrid fusion (DONE at the answer layer)** — `fuse::rrf` (Σ wᵢ/(k+rank),
    `cfg.rrf_k`) already fuses the content/reason/edge/lexical lists live in
    `answer.rs`, with a `SweepParam::RrfK` bench knob. *Remaining:* the dense seed
    merge `merge_hits` still blends raw scores (`0.4c+0.6g`) — fragile across
    scales; move it onto RRF too (+sparse lists later).
B3. **Filtered ANN end-to-end** — single source of truth: the **Tier-0** checkbox
    "Wire `search_all_filtered` into the live query path". `search_all_filtered`
    exists + is tested (`base/search.rs`) but `seed.rs` still calls the unfiltered
    `search_all_unlocked`; wiring it kills post-filtering's fewer-than-k bug on
    sparse matches. (Status tracked once, in Tier-0, so the two can't drift.)

### Stage C — Robustness / durability (don't lose data, recover fast)
C1. **Snapshots / restore** — reuse `persist::save_all` under a read lock into a
    timestamped dir + `manifest.json`; `restore` = validate + load_dir + atomic
    swap. No second persistence path.
C2. **WAL** — append-only op-log + replay-on-start over the bincode shards for
    ordered crash recovery + per-point versioning.
C3. **Memmap tiering + segment optimizer** — `diskann.rs` memmap + `cold.rs`/
    `heat.rs` tiers exist; add lazy cold→hot promotion + background compaction so
    startup and RAM scale sub-linearly with corpus size.

### Stage D — Surface & access (reach parity on interface)
D1. **REST API (axum already serving)** — the viewer runs an axum server today
    (`viewer/mod.rs`: local `/graph` `/ask_retrieve` `/edit` `/tool`; hub `/ask`
    `/tool`). The task is a first-class REST surface over the full
    `tools::dispatch`, not viewer-scoped handlers. ⚠️ duplication watch: local +
    hub already each carry a `/tool` route alongside MCP dispatch — REST + MCP
    MUST share one `tools::dispatch` core (don't add a third copy).
D2. **Auth** — API key / JWT-RBAC / TLS on the REST surface.
D3. **gRPC + SDKs**, then **multitenancy** (per-tenant graph mux / tenant filter).

### Stage E — The heavy climb (largest moat, biggest lift)
E1. **Typed payload field indexing** (keyword/int/float/bool/geo/datetime) +
    range/geo/nested filter predicates on the during-traversal `keep` fn.
E2. **Sparse vectors (SPLADE)** + **multi-vector/ColBERT** (MaxSim) +
    **product quantization** (codebook + ADC).
E3. **Distributed sharding (Raft)** + **replication/consistency factor** over the
    existing gossip layer — the one true architectural lift.
E4. **GPU index build** — gated by the 8 GB GPU already owned by Ollama; lowest
    feasibility, schedule last.

### Always-on gate
- **Benchmark harness first (Tier 0):** `retrieval_bench` → recall@k + p50/p95/p99
  + RPS + RAM vs real Qdrant on a shared dataset. Measure every stage; never claim
  "no delay" or "parity" without the number.

**Repo-law flags carried by this roadmap:** (1) B1 touches bincode-positional
`QuantizationMode`/`QuantizedVec` — append-only, guard with a round-trip test;
(2) D1 REST must not duplicate MCP dispatch (warn-on-duplicate); (3) all stages
in-process/self-contained — no pluggable backend (no-compat law).

---

## Verification provenance

Status markers below were checked against the **working tree atop `9683c5c`**
(2026-06-10; ~76 modified tracked files uncommitted). Re-stamp when re-verifying.

| Claim | Verified against (symbol) |
|---|---|
| RRF / hybrid fusion (🟡 / B2) | `fuse::rrf` (`retrieval/fuse.rs`), live at `answer.rs`; `cfg.rrf_k`, `SweepParam::RrfK` |
| A1 `answer:false` sub-ms path (DONE) | `answer_llm_args` + test `answer_false_passes_no_llm_or_embedder` (`tools_query.rs`) |
| A3 latency bundle (PARTIAL) | `complete_stream`/`params.stream`, `*_NUM_CTX`, `*_KEEP_ALIVE`, `num_gpu:0` (`llm.rs`) |
| Tier-0 profiling (DONE) | `profile_cmd` / `kern profile` |
| `search_all_filtered` NOT wired | defined+tested `base/search.rs`; `seed.rs` still calls `search_all_unlocked` |
| Snapshots / WAL / restore = ❌ | no backup/restore/WAL fns in `persist.rs` |
| Recommend/Discover/distance-matrix = ❌ | no such fns repo-wide |
| REST surface qualifier | viewer axum routes (`viewer/mod.rs`) + `kern_rpc`; no public REST/gRPC/SDK |
| Test surface | 562 `#[test]`/`#[tokio::test]` fns across 114 files (pass count needs a fresh `cargo test`) |

*Unverified (needs a run, not a grep):* live `cargo test` pass count; actual
recall@k / p50-p99 latency vs Qdrant (the Tier-0 harness, still ❌).
