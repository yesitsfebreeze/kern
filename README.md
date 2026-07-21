# kern

**A self-learning memory daemon for AI agents.** One long-running process per
working directory owns a knowledge graph that takes in durable facts from your
sessions, keeps itself small without gardening, and serves the right context
back when you need it.

kern is not a vector store you bolt onto an app. It is a *memory substrate*: it
learns on its own, compacts on its own, and (optionally) federates across
machines on its own.

```
MCP ingest ─────────────────────► typed claims ─┐
.kern/intake/ drop → distill (LLM) → typed claims ┴→ graph → recall
```

---

## What it does

- **Two ways in, both caller-driven.** An agent calls the `ingest` MCP tool to
  store a durable fact directly — the primary path. Or drop a conversation delta
  (a `.txt` file) into `<cwd>/.kern/intake/` — the daemon drains it and runs one
  LLM distillation pass that pulls out durable *facts*, *decisions*, and
  *preferences* as typed claims and ingests each into the graph. The drop dir is
  agent-agnostic: your agent, a wrapper, or a script writes it — kern ships no
  writer of its own, and captures no session automatically. Nothing is lost on an
  LLM outage — a queued delta stays until it succeeds.

- **Recalls into context.** Recall is the `query` MCP tool: relevance-targeted
  against the live graph, with provenance on every result.

- **Compacts itself.** Every access deposits a **heat** trace, and the tick's
  pulse re-deposits heat on entities still reachable from the roots; heat then
  decays lazily with age (half-life based), not per tick. A stigmergy GC evicts
  cold, stale, non-durable thoughts (Facts are immune) and spills them to a
  capped cold tier before dropping them — a latest-wins keyed table holding the
  newest 50k entries, so recent evictions stay recoverable while the very oldest
  eventually age out. Similar thoughts cluster into child kerns. The hot graph
  stays small; the long tail stays cheap.

- **Remembers across time.** Knowledge carries a bi-temporal window. When a new
  claim updates or contradicts a stored one of the same kind, kern supersedes the
  old rather than deleting it — the invalidated revision stays as history, stamped
  with when it stopped being true. A `query` can ask `as_of` a past instant to
  recover what was believed then, or `include_history` to follow the supersede
  chain back through prior revisions. The classification runs in the background
  tick, so recall stays LLM-free.

- **Federates (opt-in).** Multiple nodes share knowledge over LAN gossip with no
  coordinator. Each node heartbeats peers and merges entity bodies via a
  content-addressed CRDT — a thought ingested on node A becomes searchable on
  node B under the same content-hash id. Manually seeding `peers` is the
  reliable path today; multicast discovery only pairs nodes that share the same
  `network_id`. Off by default.

- **One graph per directory.** The daemon is per-cwd. Each project gets its own
  isolated memory; no cross-project contamination, multiple daemons per host.

---

## How it works

### The graph

kern stores two things:

- **Thoughts** — factual chunks and LLM-extracted claims. Typed (`normal`,
  `fact`, `document`) and weighted by confidence + heat.
- **Reasons** — justified edges between thoughts. The *why* connecting two
  facts, not just a similarity score.

Ids are **content hashes**, so identical content is the same node everywhere —
existence is a set union, which is what makes conflict-free merge across nodes
work.

### Retrieval

A query runs a hybrid pipeline, all hand-rolled, dependencies deliberately
minimal:

1. **Seed** — vector (HNSW) + lexical (BM25) candidate generation. The dense
   score blends the content vector with a **GNN** vector (0.4/0.6) that a
   background tick keeps re-embedding from graph structure.
2. **Expand** — walk reason edges out from the seeds; optionally **HyDE** a
   hypothetical answer to broaden recall.
3. **Fuse** — reciprocal-rank fusion of the vector and lexical lists, with
   PageRank centrality weighting the fused seeds.
4. **Rerank** (optional) — an LLM reranker reorders the head of the list.
5. **Diversify** — drop near-duplicates so the `k` results actually differ.
6. **Answer** (optional) — synthesize an LLM answer over the top results.

Cold-store results fill remaining slots (marked `cold:true`) when the hot graph
returns fewer than `k`.

### The daemon

`kern --daemon` exposes its surface two ways:

- **MCP** (stdio + HTTP/SSE) for external clients.
- **tarpc `KernRpc`** over a per-cwd socket for other local clients.

A background **tick** (default 60s) drives decay, eviction, and clustering — an
idle daemon still maintains itself. Persistence is **LMDB** (via
[heed](https://github.com/meilisearch/heed)) — an ACID, multi-process embedded
KV. Hot graph and cold tier live together in one LMDB environment
(`data.mdb` + `lock.mdb`) per data dir; vectors are stored int8, values are
`zstd(bincode)`. LMDB is single-writer: readers never block, writers serialize,
and a guarded-flush protocol keeps a stale in-memory snapshot from overwriting
newer on-disk state. HNSW, the GNN, beam search, gossip, and the MCP server
are all written from scratch.

### The hub

One machine-level supervisor owns node lifecycle. `kern mcp` asks the hub for
the project's daemon — auto-starting the hub if none runs (`[hub] auto_start =
false` opts out) — and the hub spawns the node, adopts an externally started
one, or hands back the live socket. `kern hub status` lists
tracked nodes; `kern hub unload [root]` shuts one down gracefully
(save-then-exit over RPC). Nodes idle past `--idle-unload-secs` (default 30
min) are unloaded automatically and respawn on the next connect, so memory
tracks the active set, not the installed set. `kern hub merge <src> <dst>`
folds one project's graph into another (offline CRDT union; src untouched);
`kern hub stop` ends the hub, leaving nodes up. The data path stays direct
client→daemon — the hub is connect-time only. If the hub is disabled or
unreachable, everything falls back to the pre-hub behavior.

---

## Using it

### Quickstart

**Prerequisites:** a local [Ollama](https://ollama.com) with the default
models pulled:

```bash
ollama pull qwen3-embedding:0.6b  # embeddings (default)
ollama pull granite4:3b       # distillation / reasoning (default)
# the /ask oracle answer model defaults to the same granite4:3b
```

**1. Install the binary.** A prebuilt binary for your platform (built by CI and
published to GitHub Releases):

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/yesitsfebreeze/kern/master/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/yesitsfebreeze/kern/master/install.ps1 | iex
```

> Or build from source (needs a Rust toolchain): `cargo build --release` →
> `target/release/kern`, or `cargo install --path .`.

**2. Register the MCP server with your client.** `kern mcp` attaches to a
running daemon if one exists, and otherwise auto-spawns a detached daemon for
the current directory — so this one command is all you need to bring kern up
(the installer prints the exact path). Add it as a stdio MCP server in your
client's config:

```json
{
  "mcpServers": {
    "kern": { "command": "kern", "args": ["mcp"] }
  }
}
```

**3. Wire the intake (optional).** kern is agent-agnostic: any tool that
writes a conversation delta to `<cwd>/.kern/intake/*` feeds the intake. Wire it
whatever way your client supports (a hook, a wrapper, or a manual `kern ingest`).
Recall needs no wiring — it is the `query` MCP tool.

**4. Opt the project in.** No config file is needed — every default (embedding,
reasoning, intake, tick) works out of the box against a local Ollama. The
daemon gates on the `.kern/` directory: it is created automatically the first
time the daemon persists, or `mkdir .kern` to opt in immediately. Once it
exists, the intake and recall activate for that project. (A
`<cwd>/.kern/kern.toml` is only for overriding defaults — see *Configure*
below.)

**5. Seed the graph** (see *Seed the graph* below), then start a session. From
then on, store facts by calling the `ingest` MCP tool (or drop transcripts into
`.kern/intake/`), and pull them back with `query`. kern captures nothing on its
own — the writes are yours to make.

To verify it's working, call the `health` MCP tool from your session. Prefer the MCP tools
over the `kern <subcommand>` CLI for live state — the CLI reads the on-disk
graph directly and can race the running daemon.

**Upgrading from the legacy file-shard store?** Earlier builds persisted each
kern as a separate bincode shard in `.kern/data/`. Run `kern migrate` (with the
daemon stopped) once per data directory to import them into the new LMDB store:

```bash
kern migrate              # migrates <cwd>/.kern/data/ in-place
kern migrate /dir         # or target a specific data directory
```

The old shard files are left in place; remove them once you've verified recall is
working. New projects need no migration — the LMDB store is created automatically.

### Configure

Configuration is **optional** — with no config file at all, kern ingests and
recalls with the defaults shown below. To override, create
`<cwd>/.kern/kern.toml` (project scope) or `<XDG_CONFIG>/kern/kern.toml`
(user scope; project sections win):

```toml
[reason]
# LLM for distillation. Local Ollama.
url = "http://localhost:11434"
model = "granite4:3b"       # default (small, fast, reliable)

[embed]
# Embedding model. Local Ollama.
url = "http://localhost:11434"
model = "qwen3-embedding:0.6b"  # default; dimension locks the graph (use `kern reembed` to switch)

[answer]
# User-facing /ask oracle (streamed answer over MCP). Latency-critical, only
# glues retrieved nodes into prose → smallest model that grounds. Uses Ollama's
# native /api/chat (capped context, kept GPU-resident) only when the url is local
# and does not end in /v1 — otherwise it speaks OpenAI-compatible /v1/chat/completions,
# so a remote or vLLM endpoint works here too. url/key blank → fall back to
# [reason]'s endpoint, so a single local Ollama needs no extra wiring.
model = "granite4:3b"       # default (same as [reason])

[intake]
enabled = true          # self-learning (ON by default; set false to opt out)

[tick]
interval_secs = 60      # self-compaction cadence (0 = event-driven only)

[gossip]
enabled = false         # self-distribution (opt-in)
addr = "0.0.0.0:7400"
discovery = true
discovery_port = 7475
# network_id = "team-alpha"  # optional shared discovery pool id (omit for an isolated per-daemon id)
peers = []
```

> **Before enabling gossip**, read the
> [Security](https://yesitsfebreeze.github.io/kern/concepts/security) page —
> the full trust model, including exactly what a malicious peer can and cannot
> do. Federation is unauthenticated and unencrypted today: enable it only on a
> network segment where you trust every host.

### Intake & recall

kern is agent-agnostic. There is no client-specific plugin; you wire the intake
and recall to whatever you use via two simple files.

- **Intake** — drop a conversation delta as a `.txt` file in
  `<cwd>/.kern/intake/`. The daemon drains it, distills typed claims out of it,
  and ingests them. Write the file however your client supports (a hook, a
  wrapper, or a manual `kern ingest`); kern only cares about the file.
- **Recall** — call the `query` MCP tool. It is relevance-targeted against the
  live graph and keeps provenance on every result.

Both no-op outside a directory with a `.kern/` folder, so a single global
registration is safe across every project — only directories where a kern is
(or has been) active get touched.

**Requirements:** the `kern` CLI on `PATH` and a running embedding endpoint
(Ollama by default) for recall.

### Seed the graph

Once, via the MCP tools against the running daemon (not the CLI, which races the
daemon). From an MCP session in the project:

1. Add a few gravitons — call `graviton` (action `add`) with a `name` and a
   `text` for each focus area the graph should gravitate around, e.g.
   *"decisions"*, *"project state"*, *"preferences"*. The text can be a one-line
   description or a full document/message — it is embedded whole as the
   graviton's pull vector. An optional `mass` (default `1.0`) makes a graviton
   pull harder: ingest routes by `distance / mass`, and query ranking boosts
   thoughts near a graviton by `gravity_weight * mass * cos`. Memories that
   match no graviton land in `generic`; dense `generic` clusters auto-promote to
   new gravitons over time.
2. Add the typed descriptors you want the intake to emit — call `descriptor` (action
   `add`) once each for the kinds you use: `preference`, `decision`, `project`,
   `fact`, `code-fact`, `reference`, `procedural`.

After seeding, populate the graph by calling the `ingest` MCP tool during a
session, or by dropping transcripts into `.kern/intake/`. kern ships no session
hook — the write is always a caller's call.

### MCP tools

| Tool | Purpose |
| ------ | --------- |
| `query` | Search the graph. Scored thoughts + optional LLM answer. Filter by `mode`, `kind`, `source`, time range, `min_conf`, and `as_of` (bi-temporal point-in-time); set `include_history` to also return superseded revisions (flagged `history:true`). |
| `ingest` | Add text. Supports `object_id` update semantics and `descriptor` chunking context. |
| `link` | Create a reason edge between two thoughts (LLM writes the reason if blank). |
| `forget` | Remove a thought and cascade its edges. Facts are immune. |
| `degrade` | Down-weight the edges along a bad retrieval path — teaches the graph from miss feedback. |
| `graviton` | Manage gravitons (named focus attractors; replaced the single per-kern "purpose"): `list` (default), `add` (name + text — phrase or full document — + optional mass), `remove` (name). |
| `descriptor` | Add/remove a data-type descriptor. |
| `health` | Graph stats: thought/edge counts, tick heat. |
| `pulse` | Trigger a clustering pass across the kern tree. |
| `gc` | Reap empty/orphan kerns from the running daemon's graph and persist, live. Returns before/after kern counts and the `data.mdb` size. |

---

## kern vs. traditional RAG

Traditional RAG is a pipeline you operate: chunk documents, embed them, stuff a
vector DB, and on every query do top-k cosine + prompt-stuff. kern is a memory
that operates itself.

| | Traditional RAG | kern |
| --- | --- | --- |
| **Ingestion** | Manual: you run a chunk-and-embed job over a corpus. | Caller-driven: an agent calls `ingest`, or a dropped transcript distills into typed claims via the intake — no re-indexing job. |
| **Unit stored** | Raw text chunks. | Distilled facts/decisions/preferences + *reason edges* between them. |
| **Retrieval** | top-k vector similarity. | Hybrid vector + BM25 with GNN-blended seeds, edge expansion, RRF + PageRank fusion, optional LLM rerank, diversify. |
| **Structure** | A flat bag of vectors. | A knowledge graph — recall can follow *why* one fact connects to another. |
| **Growth** | Index grows unbounded; you re-index and prune by hand. | Self-compacting: heat decay + stigmergy GC + clustering keep the hot graph small; a capped cold tier preserves the recent tail. |
| **Staleness** | Stale chunks linger until you rebuild. | Cold, non-durable thoughts decay and evict on their own; Facts persist. |
| **Feedback** | None — a bad chunk keeps ranking. | `degrade` down-weights bad retrieval paths; access heat re-ranks what you actually use. |
| **Conflicts / sync** | Single store; multi-node needs external infra. | Content-addressed CRDT + gossip; nodes converge with no coordinator. |
| **Scope** | One global index. | One graph per working directory. |

The short version: RAG gives you **search over a corpus you maintain**. kern
gives you **memory that maintains itself** — it decides what is durable, forgets
what isn't, and connects facts with reasons instead of leaving you a flat list
of nearest neighbors.

---

## Status

Self-learning and self-compaction run today. Self-distribution is wired and
enableable, but narrower than the design: entity-body sharing is verified on a
single host with manually seeded `peers` (the reliable path); multicast
auto-discovery only pairs nodes sharing the same `network_id`; and the
Delta/Question/Pulse message kinds plus the fetch RPC are handled on receipt
but have no live senders yet. Federation tuning at scale (batch size, push vs.
pull, anti-entropy) is open. Version stays `1.0.0`.
