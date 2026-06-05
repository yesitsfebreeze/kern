# The Memory Bank

`kern` is a self-learning, self-compacting, (optionally) self-distributing
memory substrate. It captures durable knowledge from your work, keeps the hot
graph small on its own, and serves recall back into context — replacing
ad-hoc file-memory / vector-store add-ons.

This guide covers what it does, how to turn it on, and where the edges are.

## The three properties

### Self-learning — capture → distill → recall

A long-running `kern` daemon owns one knowledge graph per working directory.
Knowledge flows in automatically:

```
session text → spool file → distill (LLM) → claims → graph → digest → recall
```

- A **Stop hook** (`kern-capture.mjs`) extracts the new conversation delta
  from the Claude Code transcript (user prompts + assistant text only) and
  writes it to `<cwd>/.kern/capture/`.
- The daemon's **capture spool** (`ingest::capture_spool`) drains each delta,
  runs **distillation** (`ingest::distill`) — one LLM pass that extracts
  durable facts / decisions / preferences as typed claims — and ingests each
  through the canonical `Worker`. A delta is archived to `capture/done/` only
  after every claim ingests; on LLM outage it stays for the next drain, so a
  transient failure never loses knowledge.
- The daemon keeps a **recall digest** (`retrieval::digest`) fresh at
  `<cwd>/.kern/digest.md` — the root purpose plus the hottest distilled
  thoughts. A **SessionStart hook** (`kern-recall.mjs`) injects it into each
  new session. For mid-session deep recall, the model calls the `query` MCP
  tool directly.

Both hooks fail open: if the daemon or its LLM is down, the session proceeds
normally and capture simply queues.

### Self-compacting — heat, decay, eviction, clustering

The graph stays small without manual gardening:

- Access leaves a **heat** trace; heat **decays** on each tick.
- **Stigmergy GC** evicts cold, stale, non-durable thoughts (Facts are
  immune). Cold duplicate claims fade on their own over time.
- **Clustering** consolidates similar thoughts into child kerns.

An **autonomous maintenance tick** (`[tick] interval_secs`, default 60s)
drives all of the above on a timer — an idle daemon still decays, evicts, and
clusters. Set `interval_secs = 0` to make compaction event-driven only.

### Self-distributing — gossip federation (opt-in)

Multiple nodes can share knowledge over LAN gossip with no coordinator. Each
node binds a TCP listener, heartbeats peers, and (optionally) auto-discovers
same-network peers via UDP multicast. **Off by default.**

## Turning it on

Everything is controlled from `<cwd>/.kern/kern.toml`:

```toml
[reason]
# LLM for distillation. Local Ollama. Default qwen2.5; larger models are sharper.
url = "http://localhost:11434"
model = "qwen2.5"

[capture]
enabled = true          # self-learning

[tick]
interval_secs = 60      # self-compaction cadence (0 = off)

[gossip]
enabled = false         # self-distribution (opt-in)
addr = "0.0.0.0:7400"
discovery = true
discovery_port = 7475
peers = []
```

The two Claude Code hooks are registered once in `~/.claude/settings.json`
(`Stop` → capture, `SessionStart` → recall). They are project-scoped by a
guard: they no-op in any directory without a `.kern/` folder, so a single
global registration is safe across all your projects.

Seed the graph once via MCP: set the root `purpose` and add the typed
descriptors (`preference`, `decision`, `project`, `fact`, `code-fact`,
`reference`, `procedural`).

## Status & known limits

Self-learning and self-compaction run today. Self-distribution is wired and
enableable. Recent work closed the two headline gaps:

- **Graph CRDT (implemented).** `base::merge` provides content-addressed,
  conflict-free merge of entity/edge metadata — counters join, heat /
  confidence take the max, status follows the `Active < Superseded` lattice,
  timestamps min/max. Because ids are content hashes, existence is a set
  union. Remaining: a transport that actively propagates entity bodies
  between nodes so the merge runs cross-node (today federation shares scope +
  answers + counter deltas; full entity flooding is the next wiring step).
- **Detached cold-storage tier (implemented).** Stigmergy GC spills cold,
  abandoned, non-durable thoughts to an append-only cold store
  (`<data_dir>/cold/cold.jsonl`) before dropping them from the hot graph, so
  compaction never loses data. The store self-compacts (latest-per-id) each
  GC sweep. Recall reaches it two ways: `kern get <id>` rehydrates by id, and
  the `query` tool fills remaining result slots from a cosine search over the
  cold store (marked `cold:true`) when the hot graph returns fewer than `k`.
- **Federation (verified, content-level).** `start_announce` broadcasts the
  kern's scope and `start_entity_sync` broadcasts the hottest local entity
  *bodies*; peers merge both into a per-network phantom kern via the
  content-addressed CRDT (`base::merge`) and persist them. Verified end-to-end
  on one host: two daemons bidirectionally propagate scope **and** entity
  bodies — a thought ingested on node A becomes vector-searchable on node B
  with the same content-hash id. The `kern_rpc` endpoint is now **per-cwd**
  (was per-user), so each project gets its own daemon — fixing cross-project
  memory contamination and letting multiple nodes run per host.

Near-duplicate handling relies on tighter distillation plus stigmergy GC; a
non-destructive rephrase-linking pass (`find_rephrase_candidates` +
`ReasonKind::Rephrase`) remains a future primitive. Federation tuning
(entity-sync batch size, push vs. pull, anti-entropy) is open for scale, but
the convergence path is proven.
