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
| Capture style | **Distilled, LLM-gated** — a new kern-side `ingest/distill.rs` extracts durable facts/decisions/preferences before placing. |
| Recall in CC | **Auto-inject a daemon-written digest at SessionStart + live `query` MCP tool** for deep recall mid-session. |

## Critical implementation constraints (discovered during planning)

1. **CLI subcommands race the daemon.** `kern ingest` / `query` / `purpose` /
   `descriptor` operate on the on-disk graph via `load_graph` / `save_graph`
   (see `src/commands/ingest_cmd.rs`, `src/commands/admin.rs`). The daemon is
   the single in-memory owner and overwrites the file on its next save — a CLI
   write while the daemon runs is silent data loss. **Hooks must never shell to
   a graph-touching CLI.** All capture and recall is **file-mediated through
   the daemon.**
2. **`synthesis.rs` is not an extraction pass.** It is only
   `find_rephrase_candidates` (a tick-time dedup-merge helper). The ingest
   `Worker.process()` does split → place_document → embed → place_chunks; there
   is no durable-fact gate. Distillation is therefore **new code**
   (`ingest/distill.rs`), not reuse.
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

- New **Stop hook** (`capture-claude-session`). Reads the session transcript
  (`~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`) from the last recorded line
  offset, extracts the conversation delta — user prompt strings + assistant
  `text` blocks only (drop `thinking`, `tool_use`, `tool_result`, `system`,
  attachments) — and writes it as a plain-text delta file into a watched spool
  dir, `<cwd>/.relay/capture/`. The hook does **no** LLM work and **no** graph
  access; it is a deterministic transcript-to-text extractor.
- New daemon task **`ingest/capture_spool.rs`**. Polls `.relay/capture/`, and
  for each new delta file: calls `ingest/distill.rs` (LLM extraction via the
  kern `reason` client) to turn the conversation into durable claims, enqueues
  each claim through the canonical `Worker`, then archives the consumed file to
  `.relay/capture/done/`. Archiving = natural idempotency (no re-read of a
  rewritten file). Single graph owner = the daemon, so no race.
- **`ingest/distill.rs`.** `distill(conversation, &LlmFunc) -> Vec<Claim>`
  where `Claim { text: String, kind: ClaimKind }`. The prompt asks for durable,
  reusable facts/decisions/preferences as a JSON array, explicitly skipping
  greetings, one-off task mechanics, and ephemeral chatter; returns `[]` when
  nothing is worth keeping. Each claim maps to a descriptor and is ingested as
  a thought.
- **Feedback-loop guard.** Claims are ingested with
  `Source::Session { session_id = claude-<session> }`. The Stop hook excludes
  assistant text that is verbatim kern tool output, and the SessionStart digest
  is injected as `system` context (never `user`/`assistant` text), so it is
  naturally outside the captured delta. Mirrors the existing `session_mirror`
  "drop kern-produced entries" fix (commit `7cffc24`).
- **Offset tracking.** The hook persists the last-processed line count per
  transcript in `.relay/capture/.offsets.json` so each run captures only the
  new delta and is idempotent across runs.

### 3. Capture — agnt side (native)

- `session_mirror` (Slice K) already tails the shared journal for
  `ForkOpen` / `ForkResume` / `ForkClose` and ingests each fork through the
  canonical `Worker`. Verify it is enabled in the daemon run path.
- Ensure sub-agent **receipts** route their distilled "what was learned"
  into kern as thought content (per OVERVIEW). Mostly exists — verify + wire.

### 4. Recall — Claude Code side

- New daemon task **digest writer**. On a timer (and after ingest), regenerates
  `<cwd>/.relay/kern/digest.md` = the root purpose plus the top-K hottest /
  most-recent thoughts. Because the kern is per-cwd, this digest is already
  project-scoped. Writing a file (not answering a live query) keeps recall
  decoupled from the hook and trivially fail-open.
- **SessionStart hook** (`recall-kern-digest`). `cat`s `.relay/kern/digest.md`
  and emits it as additional context (same injection mechanism context-mode /
  caveman already use). Replaces the `MEMORY.md` injection. Missing file →
  empty output, session proceeds normally.
- **Live `query` MCP tool.** Already registered on the kern MCP server — the
  model calls it mid-session for deep recall. This is the only live-query path;
  it goes through the daemon's MCP surface, never the racing CLI.

### 5. Recall — agnt side (native)

- agnt `pre_turn` already pulls context from kern before each LLM call.
  Exists; no new work.

### 6. Cutover (hard cut)

- `~/.claude/settings.json` → register the two new hooks (`Stop` →
  capture, `SessionStart` → recall) alongside the existing context-mode
  SessionStart entry, and set `enabledPlugins` `vicky@stack`,
  `context-mode@stack`, and `context-mode@context-mode` to `false`.
- File-memory: retire the `memory/` directory and add a CLAUDE.md directive
  ("memory lives in kern; do not use file-memory").
  **Caveat:** native CC file-memory is a harness built-in, not a plugin — it
  has no on/off switch. With the dir retired and `MEMORY.md` empty, the
  injection is inert, but this is the one place "hard cut" is soft rather
  than a code-level disable.

## Data flow

- **Write.** Turn ends → Stop hook extracts conversation delta → writes
  `.relay/capture/<session>-<n>.txt` → `capture_spool` task picks it up →
  `distill` (LLM) → claims → `Worker` (chunk → embed → place + propose reason
  edges) → archive the delta file. Tick decays heat; condensation parks cold
  clusters; `find_rephrase_candidates` merges near-duplicates over time.
- **Read.** Daemon digest writer keeps `.relay/kern/digest.md` fresh → session
  starts → SessionStart hook cats it → injected as context. Mid-session → model
  calls the `query` MCP tool.

## Error handling

- **kern daemon down → fail-open.** SessionStart emits nothing (session
  proceeds with no digest). Stop spools the transcript delta to a local file
  for the next successful run. A hook must never crash or block a CC session.
- **Ingest failure** is fire-and-forget already — log, do not block.
- **Offset corruption** → fall back to re-reading from 0; dedup absorbs the
  replay.

## Testing

- **kern — distill:** unit test `distill()` with a stubbed `LlmFunc` that
  returns a fixed JSON array; assert claims parse, bad JSON yields `[]`, and
  empty conversation yields `[]`.
- **kern — capture_spool:** test that a delta file dropped in the spool dir is
  distilled, enqueued, and archived to `done/`; re-running does not re-ingest.
- **kern — digest writer:** seed a graph with a purpose + thoughts; assert the
  written `digest.md` contains the purpose and the hottest thought, capped at K.
- **hook — capture:** Node test feeding a fixture transcript; assert the delta
  file contains user + assistant text only (no thinking/tool blocks) and the
  offset advances so a second run emits nothing.
- **hook — recall:** assert the hook prints digest contents when present and
  empty when the file is missing (fail-open).
- **E2E:** seed → drop a delta stating a durable fact → confirm `kern health`
  entity count rises, the fact appears in `digest.md`, and a re-drop of the
  same delta adds no new entity.

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
