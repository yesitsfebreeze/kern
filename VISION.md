# Vision

kern is a self-learning memory substrate for AI agents: one daemon per working
directory owns a knowledge graph that captures durable facts from sessions on
its own, keeps itself small without gardening, serves the right context back at
recall time, and optionally federates across machines — local-first,
self-contained, in-process, no cloud, no query-time LLM required. It is not a
vector store you operate (chunk, embed, index, prune); it is a process that
operates itself, defined by four autonomous properties — self-learning,
structured, self-compacting, self-distributing — each the inverse of a RAG
chore you'd otherwise own. The competitive set is agent memory (Zep/Graphiti,
Mem0, Letta), not general-purpose vector databases. Prose and rationale:
`docs/vision.md`, `docs/aspiration.md`.

## The test

A change fails the vision if it breaks any of these:

- **Capture is a byproduct of working.** A session's durable facts land in the
  graph with no manual ingestion step; an LLM outage queues, never loses.
- **A graph, not a bag.** Storage is typed thoughts plus reason edges — the
  *why* connecting facts, not a similarity score — and recall can walk them.
  Ids are content hashes, so identical content is the same node everywhere.
- **Superseded, never deleted.** An updated or contradicted claim becomes
  bi-temporal history queryable `as_of` a past instant; the
  update-vs-contradiction call runs off the recall path.
- **Default recall touches no LLM.** `answer:false` stays on the sub-ms graph
  path, offline-capable against local models only.
- **The hot graph stays bounded.** Decay + GC compact without intervention;
  Facts are never auto-forgotten; evictions spill to the cold tier before they
  drop.
- **Retrieval learns from use.** Access heat re-ranks what is actually used;
  `degrade` down-weights bad paths — a bad result never keeps ranking
  unpunished.
- **Fail open.** Capture and recall no-op on any error or outage; a session
  always proceeds.
- **All-internal.** No pluggable or fallback backend, no network hop on the hot
  path, dependencies deliberately minimal.
- **Federation is opt-in and coordinator-free.** Off by default; when on,
  nodes converge over gossip via the content-addressed CRDT — no central
  server.
- **One dispatch core.** Every surface (MCP, RPC, CLI, any future one) goes
  through the single `tools::dispatch` — never a second copy.
- **Claims are measured.** No SOTA, parity, or latency claim without a
  multi-seed run with error bars against the recorded baseline.
- **Per-cwd isolation.** One graph per directory; no cross-project
  contamination.
