# Vision

## What kern is

**kern is a self-learning memory substrate for AI agents.** One long-running
daemon per working directory owns a knowledge graph that captures durable facts
from your sessions, keeps itself small without manual gardening, and serves the
right context back when you need it.

It is not a vector store you bolt onto an app. A vector store is a library you
operate: you chunk, you embed, you index, you query, you prune. kern is a
process that operates *itself* — it decides what is worth remembering, forgets
what isn't, connects facts with reasons, and (optionally) shares what it learns
across machines, all on its own.

```
session text → spool → distill (LLM) → typed claims → graph → digest → recall
```

## Why it exists

AI agents are stateless between sessions. Everything an agent learns about your
project — the decisions you made, the constraints you imposed, the dead ends you
already ruled out — evaporates the moment the conversation ends. The next session
starts from zero, re-asks settled questions, and re-makes mistakes you already
corrected.

The common patch is bolt-on retrieval (RAG): dump documents into a vector DB and
prepend the top-k nearest chunks to each prompt. That helps, but it pushes all
the real work onto you and leaves structural gaps:

- **You feed it.** Ingestion is a job you run over a corpus, not a byproduct of
  working. Knowledge that only exists in a conversation never lands.
- **It only stores text.** A flat bag of chunks has no notion of *why* one fact
  relates to another, so recall can't follow a chain of reasoning.
- **It grows forever.** Stale chunks linger and keep ranking until you re-index
  and prune by hand. There is no forgetting.
- **It never learns from use.** A bad chunk that keeps surfacing keeps surfacing;
  nothing down-weights it.

kern exists to close that gap: **memory that maintains itself.** It treats
durable knowledge as a first-class, living structure instead of a static index
you babysit.

## The four properties

kern is defined by four things it does autonomously. Each is the inverse of a
RAG chore you'd otherwise own.

### 1. Self-learning — capture without a job

A Claude Code `Stop` hook extracts the new conversation delta and queues it. The
daemon drains the queue, runs one LLM distillation pass that pulls out durable
*facts*, *decisions*, and *preferences* as typed claims, and ingests each into
the graph. Recall flows back in via a fresh **digest** injected at session start,
with deeper lookups available through the `query` tool mid-session. Nothing is
lost on an LLM outage — the delta stays queued until distillation succeeds.

### 2. Structured — a graph, not a bag

kern stores two things: **thoughts** (distilled, typed, confidence-weighted
claims) and **reasons** (justified edges between them — the *why* connecting two
facts, not just a similarity score). Retrieval can walk those edges, so recall
follows a line of reasoning instead of returning a flat list of nearest
neighbors. Ids are **content hashes**, so identical content is the same node
everywhere — which is exactly what makes conflict-free merge across machines
possible.

### 3. Self-compacting — forgetting on its own

Every access leaves a **heat** trace; heat decays on every tick. A stigmergy
garbage collector evicts cold, stale, non-durable thoughts (Facts are immune)
and spills them to an append-only cold store before dropping them — so
compaction never destroys data. Similar thoughts cluster into child kerns. The
hot graph stays small and fast; the long tail stays cheap and recoverable. An
idle daemon still maintains itself on a timer.

### 4. Self-distributing — federation without a coordinator (opt-in)

Multiple nodes share knowledge over LAN gossip with no central server. Each node
heartbeats its peers and merges entity bodies via a content-addressed CRDT — a
thought ingested on node A becomes searchable on node B under the same
content-hash id. Off by default.

## Design principles

These constraints shape every decision in the codebase.

- **One graph per directory.** The daemon is per-cwd. Each project gets its own
  isolated memory — no cross-project contamination, multiple daemons per host.
- **Self-contained and in-process.** HNSW, the GNN re-embedder, beam search,
  gossip, the CRDT, and the MCP server are all written from scratch. No external
  vector DB, no network hop on the hot path. Dependencies are deliberately
  minimal.
- **No pluggable backend.** Mounting an external engine (e.g. Qdrant) as a
  backend yields a *superset*, not a replacement — it forfeits kern's only
  structural advantage (in-process, GNN vectors coupled in memory, zero network
  hop). The path forward is all-internal.
- **Never lose data on compaction.** Eviction always spills to the cold tier
  first. Facts are never auto-forgotten.
- **Fail open.** The capture and recall hooks no-op on any error or outage; a
  session always proceeds, and capture simply queues for later.

## kern vs. traditional RAG

| | Traditional RAG | kern |
|---|---|---|
| **Ingestion** | Manual: you run a chunk-and-embed job over a corpus. | Automatic: sessions distill into typed claims via a Stop hook. |
| **Unit stored** | Raw text chunks. | Distilled facts / decisions / preferences + *reason edges* between them. |
| **Retrieval** | top-k vector similarity. | Hybrid vector + BM25, edge expansion, RRF fusion, GNN + PageRank rerank, diversify. |
| **Structure** | A flat bag of vectors. | A knowledge graph — recall can follow *why* one fact connects to another. |
| **Growth** | Index grows unbounded; you re-index and prune by hand. | Self-compacting: heat decay + stigmergy GC + clustering keep the hot graph small; cold tier preserves the tail. |
| **Staleness** | Stale chunks linger until you rebuild. | Cold, non-durable thoughts decay and evict on their own; Facts persist. |
| **Feedback** | None — a bad chunk keeps ranking. | `degrade` down-weights bad retrieval paths; access heat re-ranks what you actually use. |
| **Conflicts / sync** | Single store; multi-node needs external infra. | Content-addressed CRDT + gossip; nodes converge with no coordinator. |
| **Scope** | One global index. | One graph per working directory. |

The short version: RAG gives you **search over a corpus you maintain.** kern
gives you **memory that maintains itself** — it decides what is durable, forgets
what isn't, and connects facts with reasons instead of leaving you a flat list of
nearest neighbors.

## North star

The long-range goal is to **equal or beat a dedicated vector database on its own
turf while keeping the layers it will never have.** kern already leads on graph
memory, the GNN re-embedder, self-organization, semantic caching, and LLM answer
synthesis. The open climb is the production-database tier — sharding,
replication, WAL, snapshots, on-disk tiering — built *inside* kern rather than
delegated to a backend, plus a head-to-head benchmark harness so every "parity"
and "no delay" claim is a measured number, not an assumption.

The detailed scoreboard and the staged roadmap live in
[`aspiration.md`](../aspiration.md).

## Where to go next

- **[README](../README.md)** — install, configure, and run kern.
- **[The Memory Bank](book/src/guides/memory-bank.md)** — the three properties in
  depth, and how to turn them on.
- **[Architecture](book/src/guides/architecture.md)** — the full system graph and
  its load-bearing invariants.
- **[Research & rationale](kern/README.md)** — the models and proofs behind the
  self-organizing, federated design.
- **[Federation security](FEDERATION-SECURITY.md)** — read before enabling gossip.
</content>
</invoke>
