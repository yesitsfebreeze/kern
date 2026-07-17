# Specialists

Learned expertise, written down. Each is bound by `ORACLE.md` whether run as a
subagent, persona, or read as a brief. Subagents work in-tree on disjoint file
sets; parallelize only what does not overlap.

## landscape

- **Scope:** `docs/landscape.md`, `VISION.md` (competitive-set line).
- **Knows:** the surveyed competitor set (Zep/Graphiti, Mem0, Letta, Cognee;
  YourMemory on the decay+LoCoMo axis; mnemo, AgentDB/ruvector,
  mcp-memory-service on the Rust/embedded/MCP axis; federation has papers but
  no shipped rival), which axes kern leads on feature-wise, and that no
  quality ranking is claimable until the ROADMAP #1 baseline exists.
- **Delegate when:** positioning, "how do we compare", or any claim that
  references another memory system — re-survey before trusting the doc,
  the field moves monthly.

## retrieval

- **Scope:** `src/retrieval/`, `src/gnn/`, `src/quant.rs`.
- **Knows:** the hybrid pipeline (HNSW/DiskANN + BM25 + GNN-blended seeds,
  edge expansion, HyDE, RRF + PageRank fusion, rerank, MMR diversify), int8
  quantization recall parity, the semantic query cache
  (cosine ≥ 0.97 + version stamps), filtered ANN on `is_active`.
- **Delegate when:** recall quality, ranking, ANN structure, or any change
  that could move recall@k or query latency.

## store

- **Scope:** `src/store.rs`, persistence, cold tier, `src/crdt.rs`.
- **Knows:** the single LMDB env (heed) per data dir, single-writer +
  guarded-flush protocol, `zstd(bincode)` values, the append-only-bincode law
  (persisted enums/structs grow by appending only — guard schema touches with
  a round-trip test), content-hash ids as the merge foundation, `kern migrate`.
- **Delegate when:** persistence, schema, durability (snapshots/WAL), or
  anything holding a write guard.

## lifecycle

- **Scope:** `src/tick/`.
- **Knows:** heat deposit/pulse, half-life lazy decay, stigmergy GC and its
  Fact immunity, cold-tier spill, clustering into child kerns, anchor
  auto-promotion, the bi-temporal supersede classification that runs off the
  recall path.
- **Delegate when:** decay/eviction tuning, tick cadence, or anything that
  decides what the graph forgets.

## ingest

- **Scope:** `src/ingest/`, `src/llm.rs`, `src/watcher/`.
- **Knows:** the capture spool and its outage-safe queueing, the one-pass
  distillation into typed claims, descriptors and anchors, Ollama endpoints
  (reason/embed/answer split), streaming, `num_ctx` caps, warm-keeping.
- **Delegate when:** distillation quality, claim typing, LLM latency, or
  model/endpoint wiring.

## federation

- **Scope:** `src/gossip/`, `src/trnsprt/`, `src/wire.rs`, `src/crdt.rs`
  (merge semantics), `docs/FEDERATION-SECURITY.md`.
- **Knows:** LAN gossip heartbeats, content-addressed CRDT entity-body merge,
  multicast discovery and `network_id` pairing, which message kinds have
  receivers but no senders, and that the transport is unauthenticated and
  unencrypted today.
- **Delegate when:** anything crossing a machine boundary.

## surface

- **Scope:** `src/mcp/`, `src/rpc/`, `src/commands/`, `hooks/`,
  `.claude-plugin/`.
- **Knows:** the one-dispatch-core law (every surface goes through
  `tools::dispatch`, never a second copy), the nine MCP tools, tarpc
  `KernRpc`, the three fail-open project-guarded hooks, the plugin
  marketplace packaging, and that the CLI races a live daemon (prefer MCP for
  live state).
- **Delegate when:** tool schemas, CLI subcommands, hook behavior, or plugin
  packaging.

## bench

- **Scope:** `src/bench_support/`, `src/bin/retrieval_bench.rs`,
  `src/bin/locomo_eval.rs`, `src/profile.rs`, `traces/`, `justfile`,
  `docs/kern/bench-retrieval.md`.
- **Knows:** deterministic trace generation, the nextest pipeline (host and
  container), recorded baselines, the claim standard (multi-seed, error bars,
  strict judge — leaderboard gaps under ~10 points are noise), and that the
  graph path is sub-ms while the LLM path is the delay villain.
- **Delegate when:** any performance or quality claim is about to be made —
  nothing is claimed without a run.
