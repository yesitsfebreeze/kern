# Vision

kern is a self-learning memory substrate for AI agents: one daemon per working
directory owns a knowledge graph that captures durable facts from sessions on
its own, keeps itself small without gardening, serves the right context back at
recall time, and optionally federates across machines — local-first,
self-contained, in-process, no cloud, no query-time LLM required. The
competitive set is agent memory (Zep/Graphiti, Mem0, Letta), not
general-purpose vector databases. Prose and rationale: `docs/vision.md`,
`docs/aspiration.md`.

## The test

A change fails the vision if it breaks any of these:

- **Capture is a byproduct of working.** A session's durable facts land in the
  graph with no manual ingestion step; an LLM outage queues, never loses.
- **Default recall touches no LLM.** `answer:false` stays on the sub-ms graph
  path, offline-capable against local models only.
- **The hot graph stays bounded.** Decay + GC compact without intervention;
  Facts are never auto-forgotten; evictions spill to the cold tier before they
  drop.
- **All-internal.** No pluggable or fallback backend, no network hop on the hot
  path, dependencies deliberately minimal.
- **One dispatch core.** Every surface (MCP, RPC, CLI, any future one) goes
  through the single `tools::dispatch` — never a second copy.
- **Claims are measured.** No SOTA, parity, or latency claim without a
  multi-seed run with error bars against the recorded baseline.
- **Per-cwd isolation.** One graph per directory; no cross-project
  contamination.
