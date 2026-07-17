# kern

**A self-learning memory daemon for AI agents.** One long-running process per
working directory owns a knowledge graph that captures durable facts from your
sessions, keeps itself small without gardening, and serves the right context
back when you need it.

kern is not a vector store you bolt onto an app. It is a *memory substrate*: it
learns on its own, compacts on its own, and (optionally) federates across
machines on its own.

```
session text → intake → distill (LLM) → typed claims → graph → digest → recall
```

---

## What it does

- **Captures automatically.** A Claude Code `Stop` hook extracts the new
  conversation delta and drops it in `<cwd>/.kern/capture/`. The daemon drains
  it, runs one LLM distillation pass that pulls out durable *facts*,
  *decisions*, and *preferences* as typed claims, and ingests each into the
  graph. Nothing is lost on an LLM outage — the delta stays queued until it
  succeeds.

- **Recalls into context.** The daemon keeps a fresh **digest** (root anchors +
  hottest thoughts) at `<cwd>/.kern/digest.md`. A `SessionStart` hook
  injects it into every new session. For deeper mid-session lookups the agent
  calls the `query` MCP tool directly.

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

- **MCP** (stdio + HTTP/SSE) for external clients like Claude Code.
- **tarpc `KernRpc`** over a per-cwd socket for other local clients.

A background **tick** (default 60s) drives decay, eviction, and clustering — an
idle daemon still maintains itself. Persistence is **LMDB** (via
[heed](https://github.com/meilisearch/heed)) — an ACID, multi-process embedded
KV. Hot graph and cold tier live together in one LMDB environment
(`data.mdb` + `lock.mdb`) per data dir; vectors are stored int8, values are
`zstd(bincode)`. LMDB is single-writer: readers never block, writers serialize,
and a guarded-flush protocol keeps a stale in-memory snapshot from overwriting
newer on-disk state. The recall hook never opens the store at all — it only
reads `.kern/digest.md`. HNSW, the GNN, beam search, gossip, and the MCP server
are all written from scratch.

---

## Using it

### Quickstart

**Prerequisites:** Node.js (for the hooks) and a local
[Ollama](https://ollama.com) with the default models pulled:

```bash
ollama pull qwen3-embedding:0.6b  # embeddings (default)
ollama pull qwen2.5:7b        # distillation / reasoning (default)
ollama pull qwen3.5:4b        # /ask oracle answer model (default)
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

**2. Register the MCP server with Claude Code.** `kern mcp` attaches to a
running daemon if one exists, and otherwise auto-spawns a detached daemon for
the current directory — so this one command is all you need to bring kern up
(the installer prints the exact path):

```bash
claude mcp add kern -- kern mcp
```

**3. Install the capture + recall hooks.** The simplest path is the Claude
plugin (`/plugin marketplace add yesitsfebreeze/kern` then
`/plugin install kern@kern`), which registers all three hooks plus the MCP
server in one step. The scripts ship in [`hooks/`](hooks/); see the *Hooks*
section below for the full table and behavior. They are guarded to no-op outside
`.kern/` projects, so a single global registration is safe everywhere.

**4. Opt the project in.** No config file is needed — every default (embedding,
reasoning, capture, tick) works out of the box against a local Ollama. The
hooks gate on the `.kern/` directory: it is created automatically the first
time the daemon persists, or `mkdir .kern` to opt in immediately. Once it
exists, capture and recall activate for that project. (A
`<cwd>/.kern/kern.toml` is only for overriding defaults — see *Configure*
below.)

**5. Seed the graph** (see *Seed the graph* below), then start a session. From
then on, capture and recall are automatic.

To verify it's working, call the `health` MCP tool from your session, or check
that the daemon has written `<cwd>/.kern/digest.md`. Prefer the MCP tools
over the `kern <subcommand>` CLI for live state — the CLI reads the on-disk
graph directly and can race the running daemon.

**Upgrading from the legacy file-shard store?** Earlier builds persisted each
kern as a separate bincode shard in `.kern/data/`. Run `kern migrate` (with the
daemon stopped) once per data directory to import them into the new LMDB store:

```bash
kern migrate              # migrates <cwd>/.kern/data/ in-place
kern migrate --path /dir  # or target a specific data directory
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
model = "qwen2.5:7b"        # default (small, fast, reliable)

[embed]
# Embedding model. Local Ollama.
url = "http://localhost:11434"
model = "qwen3-embedding:0.6b"  # default; dimension locks the graph (use `kern reembed` to switch)

[answer]
# User-facing /ask oracle (streamed answer over MCP). Latency-critical, only
# glues retrieved nodes into prose → smallest model that grounds. Uses Ollama's
# native /api/chat (capped context, kept GPU-resident). url/key blank → fall back
# to [reason]'s endpoint, so a single local Ollama needs no extra wiring.
model = "qwen3.5:4b"        # default; must be an Ollama model

[capture]
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

> **Before enabling gossip**, read
> [`docs/FEDERATION-SECURITY.md`](docs/FEDERATION-SECURITY.md). Federation is
> unauthenticated and unencrypted today — enable it only on a network segment
> where you trust every host.

### Hooks

Three Claude Code hooks drive kern's automatic memory. They are plain Node ESM
scripts in [`hooks/`](hooks/) with no dependencies, and all **fail open** — any
error exits 0 and the session proceeds untouched.

| Hook | Event | What it does |
|------|-------|--------------|
| `kern-capture.mjs` | `Stop` | Extracts the new conversation delta from the transcript and writes it to `<cwd>/.kern/capture/`. The daemon drains and distills it. |
| `kern-recall.mjs` | `SessionStart` | Reads `<cwd>/.kern/digest.md` and injects it into the new session as context. |
| `kern-recall-prompt.mjs` | `UserPromptSubmit` | Demand-driven semantic recall: runs `kern search <prompt>` against `<cwd>/.kern` and injects the top scored thoughts (score ≥ `MIN_SCORE`) as context for that prompt. Hard-bounded by `TIMEOUT_MS`. |

All three are **project-scoped by a guard**: each no-ops in any directory
without a `.kern/` folder, so a single global registration is safe across every
project — only directories where a kern is (or has been) active get touched.
`kern-recall-prompt` embeds the prompt every turn (Ollama), so it fails open on
timeout and injects nothing rather than blocking the prompt.

**Install as a plugin (recommended).** The repo is a self-contained Claude
**plugin** and **marketplace** — install it straight from GitHub. From any
Claude Code session:

```
/plugin marketplace add yesitsfebreeze/kern
/plugin install kern@kern
```

That registers all three hooks (via `${CLAUDE_PLUGIN_ROOT}` — no machine paths)
and the kern MCP server. Restart Claude Code to load them.

**Requirements:** the `kern` CLI on `PATH` (hooks and MCP server both shell out
to it), a running embedding endpoint for `kern-recall-prompt` (Ollama by
default), and `node` on `PATH` for hook execution.

Prefer the plugin over hand-editing `~/.claude/settings.json`; enabling it wires
all three hooks plus the MCP server in one step.

### Seed the graph

Once, via the MCP tools against the running daemon (not the CLI, which races the
daemon). From a Claude Code session in the project:

1. Add a few anchors — call `anchor` (action `add`) with a `name` and a one-line
   `text` description for each top-level bucket the root should route memories
   into, e.g. *"decisions"*, *"project state"*, *"preferences"*. Memories that
   match no anchor land in `generic`; dense `generic` clusters auto-promote to
   new anchors over time.
2. Add the typed descriptors you want to capture — call `descriptor` (action
   `add`) once each for the kinds you use: `preference`, `decision`, `project`,
   `fact`, `code-fact`, `reference`, `procedural`.

After seeding, normal sessions populate the graph automatically through the
capture hook.

### MCP tools

| Tool | Purpose |
|------|---------|
| `query` | Search the graph. Scored thoughts + optional LLM answer. Filter by `mode`, `kind`, `source`, time range, `min_conf`, and `as_of` (bi-temporal point-in-time); set `include_history` to also return superseded revisions (flagged `history:true`). |
| `ingest` | Add text. Supports `object_id` update semantics and `descriptor` chunking context. |
| `link` | Create a reason edge between two thoughts (LLM writes the reason if blank). |
| `forget` | Remove a thought and cascade its edges. Facts are immune. |
| `degrade` | Down-weight the edges along a bad retrieval path — teaches the graph from miss feedback. |
| `anchor` | Manage anchors (named top-level buckets): `list` (default), `add` (name+text), `remove` (name). |
| `descriptor` | Add/remove a data-type descriptor. |
| `health` | Graph stats: thought/edge counts, tick heat. |
| `pulse` | Trigger a clustering pass across the kern tree. |

---

## kern vs. traditional RAG

Traditional RAG is a pipeline you operate: chunk documents, embed them, stuff a
vector DB, and on every query do top-k cosine + prompt-stuff. kern is a memory
that operates itself.

| | Traditional RAG | kern |
|---|---|---|
| **Ingestion** | Manual: you run a chunk-and-embed job over a corpus. | Automatic: sessions distill into typed claims via a Stop hook. |
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
