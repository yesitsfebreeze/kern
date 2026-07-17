# Landscape

The competitive set, surveyed 2026-07-17 via web search. Compare against this
list when a positioning or eval claim is about to be made. Feature comparison
only — kern has no recorded LoCoMo baseline yet (`ROADMAP.md` #1), so no
quality ranking here is a claim.

## Closest overall (graph memory, temporal, managed forgetting)

| Project | What it is | Overlap with kern | Gap vs kern |
|---|---|---|---|
| [Zep / Graphiti](https://github.com/getzep/graphiti) | Temporal knowledge graph memory; tracks fact changes over time with provenance | Bi-temporal supersede, graph structure | Cloud/service-first; needs external graph DB; query-time LLM heavy |
| [Mem0](https://github.com/mem0ai/mem0) | LLM-managed store/retrieve/forget; the LoCoMo reference point most systems benchmark against | Claim extraction, forgetting, MCP | Hosted-first; Python; LLM on the hot path |
| [Letta (MemGPT)](https://github.com/letta-ai/letta) | Stateful agent memory OS, self-editing memory blocks | Session-spanning memory, agent-facing surface | Agent framework, not embedded substrate; LLM-driven memory ops |
| [Cognee](https://github.com/topoteretes/cognee) | Self-hosted knowledge graph engine, MCP support, Apache-2.0 | KG + MCP + self-hosted | Python; pluggable backends (kern is all-internal by vision) |

## Decay / forgetting axis (kern: heat pulse, half-life decay, stigmergy GC)

| Project | Notes |
|---|---|
| [YourMemory](https://github.com/sachitrafa/YourMemory) | Ebbinghaus forgetting-curve decay; claims +16pp over Mem0 on LoCoMo. Direct rival on the eval axis — read before recording our baseline |
| MemoryBank / [MemoryOS](https://arxiv.org/html/2506.06326v1) | Research systems: decay unless reinforced |
| [Hindsight](https://arxiv.org/pdf/2512.12818) | Retain / recall / reflect architecture paper |

## Rust + embedded + MCP axis (kern's stack)

| Project | Notes |
|---|---|
| [mnemo](https://github.com/sattyamjjain/mnemo) | MCP-native Rust embedded memory DB; REMEMBER/RECALL/FORGET/SHARE; hybrid vector search; DuckDB/Postgres backends, SDKs for Python/TS/Go. Most kern-shaped surface found |
| [AgentDB / ruvector](https://github.com/ruvnet/agentdb) | Rust engine, HNSW over redb (LMDB-inspired); learns ranking from which results the agent actually used — kern's heat/pulse idea via usage feedback |
| [mcp-memory-service](https://github.com/doobidoo/mcp-memory-service) | KG + autonomous consolidation, ~5ms retrieval, local-first, Python |

## Federation axis (kern: LAN gossip + content-addressed CRDT merge)

No shipped competitor found — papers only:
[gossip substrate for agentic AI](https://arxiv.org/html/2512.03285v1),
[CRDT state sync for agent fleets](https://zylos.ai/research/2026-03-17-crdts-distributed-state-sync-multi-agent-systems/).
Federation is a differentiator, not catch-up — but it is `building`,
unauthenticated, and sender-less today, so it is not claimable either.

## Where kern stands (feature-level, unmeasured)

Held by no single competitor found: graph + decay/GC + bi-temporal + embedded
Rust/LMDB + no-LLM default recall + CRDT federation in one binary. Per-axis:
mnemo/AgentDB closest on stack, Graphiti closest on temporal semantics,
YourMemory closest on decay + published LoCoMo numbers.

Honest gaps, today:

- **No recorded eval baseline.** Mem0, Zep, YourMemory publish LoCoMo numbers;
  kern's harness runs but the blocker in `ROADMAP.md` #1 stands. Until then any
  "better recall" statement violates verify-before-claiming.
- **No durability primitive.** Snapshots/WAL undecided (`ROADMAP.md` #4);
  most competitors sit on DBs that already have one.
- **Single language surface.** mnemo ships Python/TS/Go SDKs; kern is
  MCP/RPC/CLI only (by design, but it narrows adoption).
- **Federation not claimable.** See above.
