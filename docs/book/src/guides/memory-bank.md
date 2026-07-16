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
  `<cwd>/.kern/digest.md` — the root's anchors plus the hottest distilled
  thoughts. A **SessionStart hook** (`kern-recall.mjs`) injects it into each
  new session. For mid-session deep recall, the model calls the `query` MCP
  tool directly.

The hooks fail open: if the daemon or its LLM is down, the session proceeds
normally and capture simply queues.

### Self-compacting — heat, decay, eviction, clustering

The graph stays small without manual gardening:

- Every access deposits a **heat** trace, and the tick's pulse re-deposits
  heat on thoughts still reachable from the roots; heat then decays lazily
  with age (half-life based), not per tick.
- **Stigmergy GC** evicts cold, stale, non-durable thoughts (Facts are
  immune), spilling each one to the capped cold tier first. The staleness
  clock reads the thought's last access, falling back to its creation time.
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
# LLM for distillation. Local Ollama. Default qwen2.5:7b; larger models are sharper.
url = "http://localhost:11434"
model = "qwen2.5:7b"

[capture]
enabled = true          # self-learning

[tick]
interval_secs = 60      # self-compaction cadence (0 = off)

[gossip]
enabled = false         # self-distribution (opt-in)
addr = "0.0.0.0:7400"
discovery = true
discovery_port = 7475
# network_id = "team-alpha"  # optional shared discovery pool id (omit for an isolated per-daemon id)
peers = []
```

Three Claude Code hooks drive the automatic memory (`Stop` → capture,
`SessionStart` → digest recall, `UserPromptSubmit` → per-prompt semantic
recall); the simplest install is the Claude plugin, which registers all three
plus the MCP server in one step (see the README's *Hooks* section). They are
project-scoped by a guard: they no-op in any directory without a `.kern/`
folder, so a single global registration is safe across all your projects.

Seed the graph once via MCP: add a few `anchor`s — named top-level buckets the
root routes matching memories into (anything unmatched lands in `generic`) — and
the typed descriptors (`preference`, `decision`, `project`, `fact`, `code-fact`,
`reference`, `procedural`).

## Status & known limits

Self-learning and self-compaction run today. Self-distribution is wired and
enableable, but narrower than the design. The load-bearing pieces:

- **Graph CRDT (implemented).** `base::merge` provides content-addressed,
  conflict-free merge of thought/edge metadata — counters join, heat takes the
  max, status follows the `Active < Superseded` lattice, timestamps min/max.
  Confidence is deliberately **replica-local** (never imported from a peer) so
  a compromised node cannot pin a poisoned claim's confidence high
  federation-wide. Because ids are content hashes, existence is a set union.
- **Capped cold tier (implemented).** Stigmergy GC spills cold, abandoned,
  non-durable thoughts to a cold table in the same LMDB store before dropping
  them from the hot graph — a latest-wins keyed table holding the newest 50k
  entries, so recent evictions stay recoverable while the very oldest
  eventually age out. Recall reaches it two ways: `kern get <id>` rehydrates
  by id, and the `query` tool fills remaining result slots from a cosine
  search over the cold tier (marked `cold:true`) when the hot graph returns
  fewer than `k`.
- **Bi-temporal invalidation (implemented).** When a same-kind near-duplicate
  updates or contradicts a stored claim, the background tick *supersedes* the old
  revision instead of deleting it: it flips to `Superseded`, stamps `valid_to`
  (when it stopped being true) and `invalidated_at` (when kern learned of it),
  evicts it from the ANN, and links the pair with a `Supersedes` edge. Invalidated
  history loses its GC immunity and spills to the cold tier — invalidated is not
  deleted. The `query` tool adds `as_of` (return the revision whose validity
  window covered a past instant) and `include_history` (also return superseded
  revisions reachable from the active hits, marked `history:true`). The
  update-vs-contradiction call is a background reason-LLM classification, so recall
  stays LLM-free; with no LLM configured a differing near-dup is kept as a
  `Rephrase` edge exactly as before (fail open).
- **Federation (verified, content-level).** `start_announce` broadcasts the
  kern's scope and `start_entity_sync` broadcasts the hottest local thought
  *bodies*; peers merge both into a per-network `remote-*` phantom kern via
  the content-addressed CRDT (`base::merge`), index them for vector search on
  receipt, and persist them. Verified end-to-end on one host: two daemons
  bidirectionally propagate scope **and** thought bodies — a thought ingested
  on node A becomes vector-searchable on node B with the same content-hash
  id. Manually seeding `peers` is the reliable path today; multicast
  discovery only pairs nodes that share the same `network_id`. The
  Delta/Question/Pulse message kinds plus the fetch RPC are handled on
  receipt but have no live senders yet.

Near-duplicate ingests are handled non-destructively: a duplicate above the
similarity threshold updates the existing thought's confidence and records
the alternate phrasing as a `Rephrase` edge instead of mutating the canonical
text. Federation tuning at scale (entity-sync batch size, push vs. pull,
anti-entropy) is open, but the convergence path is proven.
