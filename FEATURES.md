# Features

What exists right now. States: `building` | `active`.

- **Capture & distillation** — `Stop` hook spools the session delta; the daemon
  drains it through one LLM pass into typed claims (facts, decisions,
  preferences). `active`
- **Digest recall** — `SessionStart` hook injects `.kern/digest.md` (root
  anchors + hottest thoughts), maintained fresh by the daemon. `active`
- **Prompt-time recall** — `UserPromptSubmit` hook runs semantic search over
  the prompt and injects scored thoughts, fail-open with a hard timeout.
  `active`
- **Query pipeline** — hybrid retrieval: HNSW + BM25 + GNN-blended seeds, reason-edge
  expansion, optional HyDE, RRF + PageRank fusion, optional LLM rerank,
  diversify, optional LLM answer; cold-store backfill. `active`
- **Bi-temporal history** — contradicting claims supersede rather than delete;
  `as_of` point-in-time queries and `include_history` supersede-chain walks;
  classification runs off the recall path. `active`
- **Self-compaction** — background tick drives heat pulse, half-life decay,
  stigmergy GC (Facts immune), clustering into child kerns. `active`
- **Cold tier** — capped latest-wins table (newest 50k) catching evictions
  before they drop. `active`
- **LMDB persistence** — single ACID env per data dir, int8 vectors,
  `zstd(bincode)` values, guarded flush against stale-snapshot overwrite;
  `kern migrate` imports legacy file shards. `active`
- **MCP surface** — stdio + HTTP/SSE server exposing `query`, `ingest`,
  `link`, `forget`, `degrade`, `anchor`, `descriptor`, `health`, `pulse`; all
  through the one `tools::dispatch` core. `active`
- **RPC surface** — tarpc `KernRpc` over a per-cwd socket for local clients.
  `active`
- **CLI** — `kern` subcommands (daemon, mcp, search, profile, reembed,
  migrate, …); reads the on-disk graph directly, can race a live daemon.
  `active`
- **Claude Code plugin** — repo doubles as plugin + marketplace; registers the
  three hooks and the MCP server in one install; hooks fail open and no-op
  outside `.kern/` projects. `active`
- **Federation** — LAN gossip + content-addressed CRDT entity-body merge, no
  coordinator; reliable with manually seeded `peers`, multicast discovery only
  within a shared `network_id`; Delta/Question/Pulse kinds and fetch RPC
  handled on receipt but have no live senders; unauthenticated and unencrypted
  (see `docs/FEDERATION-SECURITY.md`). Off by default. `building`
- **Bench pipeline** — `retrieval_bench` binary, deterministic workload traces,
  single nextest pipeline (host and container), baselines in
  `docs/kern/bench-retrieval.md`. `active`
- **LoCoMo eval harness** — `locomo_eval` (feature `bench`): F1 / ROUGE-L /
  LLM-judge per category, adversarial abstention, `--json`; no recorded
  baseline yet. `building`
