# Feature Audit — FEATURES.md vs src/ code reality

Date: 2026-07-18. Method: each FEATURES.md claim traced to source. Build: `cargo check` clean (default features).

Legend: PASS = code matches claim. PARTIAL = core present, claim partly false. FAIL = claim not realized in code.

## PASS (11)

- **F4 Query pipeline** — `retrieval/{seed,expand,fuse,rerank,diversify,answer,hyde,pagerank}.rs` all present; HNSW id-stable (`base/hnsw.rs`), BM25 (`base/lexical.rs`), GNN blend (`gnn/`), RRF (`fuse::rrf`), PageRank, optional rerank/diversify/answer, cold backfill via `store::cold_search`. ✓
- **F5 Bi-temporal history** — `Entity.{valid_from,valid_until,superseded_by,invalidated_at}`, `is_superseded`, `stamp_invalidated`; `ScoreFilter{as_of,include_history}` in `retrieval/score.rs`; classification in `tick/tasks::do_classify_contradiction` (off recall path). ✓
- **F6 Self-compaction** — `tick/pulse.rs` (heat pulse + half-life), `tick/stigmergy.rs::run_gc` (Facts immune via `base/types.rs`), `tick/cluster.rs` + `tick::do_cluster` spawns child kerns. ✓
- **F7 Cold tier** — `store.rs` `COLD_DB`, `cold_spill`/`cold_get`/`cold_all`/`cold_search`, `cold_cap(COLD_MAX_ENTRIES)` (50k latest-wins). ✓
- **F8 LMDB persistence** — `base/persist.rs` + `base/store.rs`: heed env, int8 (`quant.rs`), `zstd(bincode)` (`store::bincode_cfg`), `save_graph_guarded` + `merge::absorb_graph` retry, `mutation_epoch`-gated snapshot (`graph::bump_mutation_epoch`), `base/migrate.rs::migrate_dir` imports legacy shards. ✓
- **F9 MCP surface** — `mcp/tools.rs` dispatch exposes all 9: query, ingest, link, forget, degrade, anchor, descriptor, health, pulse. stdio (`mcp::run_stdio`) + HTTP/SSE (`mcp/sse::run_sse`). ✓
- **F10 RPC surface** — `rpc/kern_rpc_server.rs` `impl KernRpc for KernRpcHandler`, per-cwd socket bind in `run_server`. ✓
- **F11 CLI** — `commands::Commands` enum: Ingest, Query, Search, Reembed, Get, List, Forget, Link, Health, Profile, Gc, Compact, Anchor, Degrade, Descriptor, Peers, Register, Unnamed, Mcp, Compress, Migrate, Daemon. Reads on-disk graph via `load_graph` (can race daemon). ✓
- **F13 Federation** (`building`) — `crdt.rs` `OrSet`/`LwwRegister`, `GraphGnn::bump_lamport`/`observe_lamport`, `PendingDelta` queue, `gossip/handler.rs::start_delta_flush` (Delta sender), Pulse + Question senders wired in `bootstrap`; LWW for `valid_until` + `Reason.score`, `statements` OR-Set union. Off by default (`GossipConfig::enabled = false`, `start_gossip` early-returns). Matches Phase-1 claim. ✓
- **F14 Bench pipeline** — `bin/retrieval_bench.rs`, `traces/workload.json` (deterministic), justfile `nextest` (host + docker container), `docs/kern/bench-retrieval.md` baseline. ✓
- **F15 LoCoMo eval** (`building`) — `bin/locomo_eval.rs` + `bench_support/locomo_run.rs`: F1 (`token_f1`), ROUGE-L (`rouge_l`), LLM-judge, adversarial abstention, `--json`. `eval/baseline/` empty = matches "no recorded baseline yet". ✓

## NOT UP TO STANDARD (4) — hook layer retired but docs not updated

Commit `483b37c` (HEAD) deleted the entire `hooks/` directory — `hooks.json`, `kern-capture.mjs` (Stop), `kern-recall.mjs` (SessionStart), `kern-recall-prompt.mjs` (UserPromptSubmit) — with the note "retired hooks/ capture scripts (superseded by MCP capture intake)". But **FEATURES.md, README.md, and `.claude-plugin/plugin.json` still claim the three hooks.** There is no MCP-based automatic replacement: `ingest` is explicit, not a Stop-hook delta intake.

- **F1 Capture & distillation** — PARTIAL. Daemon side complete: `ingest/intake.rs::run` drains `.kern/capture/`, `ingest/distill.rs::distill` produces typed claims, wired in `bootstrap::spawn_capture`. But the **Stop hook that populates `.kern/capture/` is gone** — nothing writes the session delta there. "Stop hook intakes the session delta" is false. Either restore the hook or rewrite the claim to match the (currently manual/explicit) intake path.
- **F2 Digest recall** — PARTIAL. Daemon writes `.kern/digest.md` (`spawn_capture` loop → `retrieval/digest::write_digest`). But the **SessionStart hook that injects it into the session is gone**. "SessionStart hook injects .kern/digest.md" is false; digest is written but never injected.
- **F3 Prompt-time recall** — FAIL. `kern-recall-prompt.mjs` (UserPromptSubmit semantic-search injection, fail-open timeout) deleted, no replacement. "UserPromptSubmit hook runs semantic search over the prompt and injects scored thoughts" is not realized.
- **F12 Claude Code plugin** — FAIL. `.claude-plugin/plugin.json` still has `"hooks": "./hooks/hooks.json"` but **`hooks/hooks.json` does not exist**. Plugin install registers the MCP server but cannot register the three hooks. "Registers the three hooks and the MCP server in one install" is false.

## Root cause

One commit retired the hook scripts without updating the three docs that describe them. Fix path: either (a) restore `hooks/` (revert the deletion — the scripts existed and were tested) and keep the claims, or (b) update FEATURES.md / README.md / `plugin.json` to describe whatever the intended "MCP capture intake" replacement actually is (today there is none for automatic capture/recall).

## Build

`cargo check` (default features) — Finished clean, 0 warnings.
