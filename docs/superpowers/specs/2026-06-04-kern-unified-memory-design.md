# kern as the single memory substrate — design

**Date:** 2026-06-04
**Status:** approved (brainstorm)
**Author:** febreeze + Claude

## Goal

Replace every memory system in the daily workflow — Claude Code native
file-memory, the Vicky KB, and the context-mode FTS5 store — with **kern**,
the local knowledge-graph daemon. kern becomes the one substrate; it learns
automatically from sessions and serves recall back into context without
manual curation.

## Decisions (locked during brainstorm)

| Fork | Decision |
|------|----------|
| Target consumer | **Both** — Claude Code (via MCP + hooks) and the native `agnt` loop (via tarpc). One graph, two consumers. |
| Capture style | **Distilled, LLM-gated** — extract durable facts/decisions/preferences; lean on kern's `synthesis.rs`. |
| Recall in CC | **Auto-inject digest at SessionStart + live `query` MCP tool** for deep recall mid-session. |
| Migration | **Start fresh** — no backfill of old stores; kern learns forward. |
| Cutover | **Hard cut now** — disable Vicky + context-mode immediately; retire file-memory by convention. |

## Why this works

kern already solves, by design, every flaw of the file-memory system:

| File-memory flaw | kern's built-in answer |
|---|---|
| `MEMORY.md` index grows unbounded | Condensation — cold clusters detach to sub-DBs, lazy-link on query. Hot graph stays small. |
| Manual dedup | `src/ingest/dedup.rs` in the ingest path. |
| Staleness | Stigmergic `heat`: decays on tick, reinforced on traversal; `forget` / `degrade` / `pulse`. |
| Weak recall (string match) | vector + lexical + GNN fusion (`qbst`). |
| Three competing stores | kern **is** the one substrate. |

The engine exists; the gap is that the graph is empty (0 entities, purpose
unset, 0 descriptors) and the CC-side capture/recall glue is not wired.

## Architecture

One graph (kern daemon, per-cwd, already running as an MCP server). Two
write paths in, two read paths out.

```
                 ┌──────────────── kern graph (one) ───────────────┐
                 │  thoughts + reasons · heat/decay · condensation  │
                 └──────────────────────────────────────────────────┘
   write ▲ ▲                                            read │ │
         │ └── agnt: session_mirror + receipts (tarpc)       │ └── agnt: pre_turn pull (tarpc)
         └──── CC:   Stop hook → `kern ingest` (distill)     └──── CC:   SessionStart digest + `query` tool
```

## Components

### 1. Seed (one-time)

- `kern purpose` set to the root purpose: personal + project memory for relay
  work — durable facts, decisions, preferences.
- `kern descriptor add` for the typed kinds, replacing file-memory's taxonomy
  and giving `synthesis` chunking context:
  - `preference` — how the user wants work done
  - `decision` — choices made and why
  - `project` — ongoing work / goals / constraints
  - `fact` — durable factual claim
  - `code-fact` — structural truth about a codebase
  - `reference` — pointer to an external resource

### 2. Capture — Claude Code side (distilled)

- New **Stop / SessionEnd hook**. Reads the session transcript
  (`~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`) from the last recorded
  byte offset, pipes the new turns to `kern ingest --source claude-session`.
- **Distillation lives in kern**, not the hook. The ingest `Worker` runs
  `synthesis` claim-extraction via the kern LLM client (`reason_url`), then
  `dedup`, then `place` + reason-edge proposal. The hook is a dumb pipe — one
  code path, no duplicated extraction logic.
- **Feedback-loop guard.** Tag `source=claude-session`. Exclude from capture:
  the SessionStart digest text and any kern MCP tool I/O (so the model
  relaying kern output is not re-ingested). Mirrors the existing
  `session_mirror` "drop kern-produced entries" fix (commit `7cffc24`).
- **Offset tracking.** Persist the last-ingested byte offset per transcript
  file so the hook is incremental and idempotent across runs.

### 3. Capture — agnt side (native)

- `session_mirror` (Slice K) already tails the shared journal for
  `ForkOpen` / `ForkResume` / `ForkClose` and ingests each fork through the
  canonical `Worker`. Verify it is enabled in the daemon run path.
- Ensure sub-agent **receipts** route their distilled "what was learned"
  into kern as thought content (per OVERVIEW). Mostly exists — verify + wire.

### 4. Recall — Claude Code side

- **SessionStart hook.** Queries kern for the root purpose plus top-heat
  thoughts relevant to the cwd, emits a compact digest as additional context
  (same injection mechanism context-mode / caveman already use). Replaces the
  `MEMORY.md` injection.
- **Live `query` MCP tool.** Already registered on the kern MCP server — the
  model calls it mid-session for deep recall.

### 5. Recall — agnt side (native)

- agnt `pre_turn` already pulls context from kern before each LLM call.
  Exists; no new work.

### 6. Cutover (hard cut)

- `~/.claude/settings.json` → `enabledPlugins`: set `vicky@stack`,
  `context-mode@stack`, and `context-mode@context-mode` to `false`.
- File-memory: retire the `memory/` directory and add a CLAUDE.md directive
  ("memory lives in kern; do not use file-memory").
  **Caveat:** native CC file-memory is a harness built-in, not a plugin — it
  has no on/off switch. With the dir retired and `MEMORY.md` empty, the
  injection is inert, but this is the one place "hard cut" is soft rather
  than a code-level disable.

## Data flow

- **Write.** Turn ends → Stop hook → transcript delta → `kern ingest` →
  `Worker`: chunk → `synthesis` extract claims → `dedup` → `place` + propose
  reason edges. Tick decays heat; condensation parks cold clusters.
- **Read.** Session starts → SessionStart hook → `kern query` → digest
  injected. Mid-session → model calls the `query` tool.

## Error handling

- **kern daemon down → fail-open.** SessionStart emits nothing (session
  proceeds with no digest). Stop spools the transcript delta to a local file
  for the next successful run. A hook must never crash or block a CC session.
- **Ingest failure** is fire-and-forget already — log, do not block.
- **Offset corruption** → fall back to re-reading from 0; dedup absorbs the
  replay.

## Testing

- **kern:** transcript-parse unit test (jsonl → clean turns, tool I/O
  stripped); existing distillation/dedup tests cover the rest.
- **hook:** dry-run the Stop hook against the current transcript; assert
  `kern health` entity count rises and no duplicate entities appear on
  re-run.
- **recall:** SessionStart digest snapshot test.
- **E2E:** seed purpose → run a session that states a durable fact → confirm
  the fact is recalled in the next session's SessionStart digest.

## Risks / open questions

1. **Harness file-memory not code-disablable** — neutralized by convention
   only. Accepted.
2. **Distillation cost** — one LLM call per session-end via `reason_url`;
   needs a cheap/small model to stay free-feeling.
3. **Over-extraction noise** — rely on heat-decay to prune; tune
   `dedup_threshold`.
4. **Transcript schema coupling** — CC's jsonl format may change; isolate the
   parser behind one module.

## Out of scope

- Backfill/import of existing file-memory, Vicky, or context-mode content
  (decision: start fresh).
- Federation / gossip changes — kern's existing behavior unchanged.
- Version stays 1.0.0. No compat shims.
