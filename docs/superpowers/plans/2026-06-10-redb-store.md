# redb Storage Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace kern's file-per-shard bincode persistence with a single embedded ACID KV store (redb), value = `zstd(bincode(node))`, vectors stored int8-on-disk, `gnn_vector` dropped (recomputed).

**Architecture:** Swap the *container*, not the codec. One `kern.redb` file per `data_dir`. v1 keeps the in-memory model (`Kern` holds entities inline) untouched and changes only the persist boundary: a `KERN` table maps `kern_id -> zstd(bincode(StoredKern))`, where `StoredKern` carries the kern with entity/reason vectors lifted out into int8 `QuantizedVec` side-maps and `gnn_vector` omitted. The cold tier moves from `cold.jsonl` into a `COLD` table. A one-shot `kern migrate` reads legacy `.kern` shards (legacy reader retained, fenced) and writes the new store. Entity-granular keying + per-entity persist-dirty (the write-amplification fix) is a documented **Phase 5 follow-up**, deliberately deferred so the high-value, low-risk container swap ships and is verified first.

**Tech Stack:** Rust, redb 2.x (pure-Rust embedded KV, MVCC, crash-safe), zstd 0.13, bincode 2 (serde), existing `crate::quant::QuantizedVec`.

**Scope note / honest trade:** kern-granular v1 already beats today on all four axes — reliability (redb ACID/WAL vs bare file writes), size (int8 + drop gnn_vector + zstd), scalability (mmap, no inode explosion, MVCC reclamation), and faster load/write (cursor scan vs 347k `open()`; one batched commit vs per-file fsync). It does **not** fix steady-state write-amplification (a structural change still re-encodes the touched kern). That specifically needs per-entity rows + per-entity dirty tracking = Phase 5.

---

## File Structure

- **Create** `src/base/store.rs` — redb wrapper: `Store` type, table defs, `zstd(bincode)` codec, generic `put/get/remove/scan`, plus typed `save_all`/`load_all` over `StoredKern`, cold-tier ops, and the `_meta` (network_id/quant_mode) row. One responsibility: durable storage.
- **Create** `src/base/migrate.rs` — legacy `.kern` reader (lifted from today's `persist.rs`) + `migrate_dir(old) -> Store`. Fenced as migration-only.
- **Modify** `src/base.rs` (or `src/base/mod.rs`) — register `store`, `migrate` modules.
- **Modify** `src/base/graph.rs` — `load_kern`/`save_kern`/`delete_kern` internal calls (lines 178, 308, 318) route through a `Store` handle held on `GraphGnn`.
- **Modify** `src/base/persist.rs` — gut the file-shard writers; keep only what `store`/`migrate` reuse. `save_all`/`load_dir` become store-backed.
- **Modify** `src/tick/tasks.rs:289 do_persist` — write through the store.
- **Modify** `src/tick/stigmergy.rs:82,93` — cold spill/compact through the store.
- **Modify** `src/mcp/tools_query.rs:259`, `src/commands/graph_ops.rs:62`, `src/commands/reembed.rs:108` — cold reads through the store.
- **Modify** `src/commands/admin.rs:21,256` — `compress_dir`/`load_dir` store-backed.
- **Modify** `src/commands.rs` — `load_graph`/`save_graph` open/commit the store; add `Migrate` subcommand wiring.
- **Modify** `Cargo.toml` — add `redb`, `zstd`.
- **Delete** `src/base/cold.rs` (JSONL tier) once the `COLD` table replaces it.

## Key types (defined once, referenced by later tasks)

```rust
// src/base/store.rs
use crate::quant::QuantizedVec;
use crate::base::types::Kern;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// On-disk projection of a Kern: the kern with all entity/reason vectors lifted
/// out into int8 side-maps and `gnn_vector` dropped (recomputed by GnnPropagate).
#[derive(Serialize, Deserialize)]
pub struct StoredKern {
    pub kern: Kern,                                  // entity.vector / gnn_vector / reason.vector EMPTIED
    pub entity_vecs: HashMap<String, QuantizedVec>,  // int8
    pub reason_vecs: HashMap<String, QuantizedVec>,  // int8
}
```

`StoredKern::from_kern(&Kern) -> StoredKern` clones, moves each `entity.vector` into `entity_vecs` as `QuantizedVec::encode(v, Int8)`, clears `entity.vector` + `entity.gnn_vector`, same for `reason.vector`. `StoredKern::into_kern(self) -> Kern` decodes side-maps back into `entity.vector`/`reason.vector` (f64), leaves `gnn_vector` empty.

---

## Task 1: Add dependencies (the gate)

**Files:** Modify `Cargo.toml`

- [ ] **Step 1:** Add to `[dependencies]`: `redb = "2"`, `zstd = "0.13"`.
- [ ] **Step 2:** Run `cargo fetch` — Expected: both crates resolve and download. If offline/blocked, STOP: the swap can't proceed; report to user.
- [ ] **Step 3:** `cargo build` — Expected: green (deps compile, nothing uses them yet).
- [ ] **Step 4:** Commit `feat(store): add redb + zstd deps`.

## Task 2: store.rs codec (TDD, isolated)

**Files:** Create `src/base/store.rs`; register in `src/base.rs`.

- [ ] **Step 1 (failing test):** `codec_roundtrips_a_struct` — `encode(&v)` then `decode::<T>()` returns `v`, for a small `#[derive(Serialize,Deserialize,PartialEq)]` struct with a `Vec<f64>` and a `String`.
- [ ] **Step 2:** Run — fails (no `store` module).
- [ ] **Step 3:** Implement `encode<T:Serialize>` = `[FORMAT_V1] ++ zstd(bincode(v))`; `decode<T:DeserializeOwned>` = strip version, `zstd::decode_all`, `bincode::serde::decode_from_slice`. `StoreError` thiserror enum (Io, Redb, Bincode encode/decode, Zstd, BadVersion).
- [ ] **Step 4:** Run — passes.
- [ ] **Step 5:** Add `decode_rejects_unknown_version` (first byte `0xFF` → `BadVersion`). Implement, pass.
- [ ] **Step 6:** Commit `feat(store): zstd(bincode) versioned codec`.

## Task 3: Store open + generic KV (TDD)

**Files:** `src/base/store.rs`

- [ ] **Step 1 (failing test):** `put_get_remove_roundtrip` over a temp dir: `Store::open(dir)`, `put(KERN, "k", &val)`, `get::<T>(KERN, "k") == Some(val)`, `remove(KERN,"k")`, `get == None`.
- [ ] **Step 2:** Run — fails.
- [ ] **Step 3:** Implement `Store { db: redb::Database }`, `open(dir)` → `dir/kern.redb`, table defs `KERN/COLD/META: TableDefinition<&str,&[u8]>`. `put/get/remove` open a write/read txn, encode/decode value. `scan<T>(table) -> Vec<(String,T)>`.
- [ ] **Step 4:** Run — passes.
- [ ] **Step 5:** `scan_returns_all_rows` test. Pass.
- [ ] **Step 6:** `reopen_persists_data` test (open, put, drop, reopen, get) — proves durability across handles. Pass.
- [ ] **Step 7:** Commit `feat(store): redb-backed typed KV (put/get/remove/scan)`.

## Task 4: StoredKern projection (TDD)

**Files:** `src/base/store.rs`

- [ ] **Step 1 (failing test):** `stored_kern_roundtrip_quantizes_and_drops_gnn` — build a `Kern` with one entity (non-empty `vector`, non-empty `gnn_vector`) and one reason (non-empty `vector`); `from_kern` then `into_kern`; assert entity `vector` is recovered within int8 tolerance, `gnn_vector` is empty, reason `vector` recovered, all other fields equal.
- [ ] **Step 2:** Run — fails.
- [ ] **Step 3:** Implement `StoredKern::from_kern`/`into_kern` per the Key-types section.
- [ ] **Step 4:** Run — passes.
- [ ] **Step 5:** `stored_kern_handles_empty_vectors` (entity with no vector → no side-map entry, `has_vector()==false` after round trip). Pass.
- [ ] **Step 6:** Commit `feat(store): StoredKern int8 projection`.

## Task 5: Store save_all / load_all + _meta (TDD)

**Files:** `src/base/store.rs`

- [ ] **Step 1 (failing test):** `save_then_load_graph_roundtrip` — given a `HashMap<String,Kern>` + network_id + quant_mode, `save_all_kerns` then `load_all_kerns` returns equal kerns (vectors within int8 tol) and the same network_id/quant_mode.
- [ ] **Step 2:** Run — fails.
- [ ] **Step 3:** Implement `save_all_kerns(&self, kerns, network_id, quant_mode)` — one write txn: put each kern as `StoredKern`, prune `KERN` rows not in the live set (replaces the orphan-prune in `save_all`), put `_meta`. `load_all_kerns(&self) -> (HashMap<String,Kern>, network_id, quant_mode)`. `save_one_kern`/`load_one_kern`/`delete_one_kern` for the graph internals.
- [ ] **Step 4:** Run — passes.
- [ ] **Step 5:** `save_all_prunes_removed_kerns` (save {a,b}, then save {a}, load → only a). Pass.
- [ ] **Step 6:** `corrupt_value_is_skipped_with_warn` (inject a bad `KERN` value, `load_all_kerns` skips it, others load). Pass.
- [ ] **Step 7:** Commit `feat(store): graph-level save_all/load_all over redb`.

## Task 6: Cold tier in the store (TDD)

**Files:** `src/base/store.rs`

- [ ] **Step 1 (failing tests):** port `cold.rs` semantics — `cold_spill_then_get`, `cold_latest_wins`, `cold_search_ranks_by_cosine`, `cold_cap_drops_oldest`. The `COLD` table is `id -> zstd(bincode(Entity))`; spill = put (latest wins naturally, no dup lines); `cold_search` decodes a `{id,vector}` projection? No — redb has no partial decode, so decode full Entity per row but only over COLD (bounded by cap). cap-drop on spill when `COLD` len > `COLD_MAX_ENTRIES`, dropping oldest by `created_at`.
- [ ] **Step 2:** Run — fail.
- [ ] **Step 3:** Implement `cold_spill/cold_get/cold_search/cold_cap`.
- [ ] **Step 4:** Run — pass.
- [ ] **Step 5:** Commit `feat(store): cold tier as a redb table`.

## Task 7: Hold a Store on GraphGnn; route internals

**Files:** `src/base/graph.rs`, `src/base/persist.rs`

- [ ] **Step 1:** Add `store: Option<Arc<Store>>` to `GraphGnn` (None when `data_dir` empty). Open in `from_saved_*`/`load_graph`.
- [ ] **Step 2:** Route `get` lazy-load (`graph.rs:178`) → `store.load_one_kern(id)`; `unload` (`:318`) → `store.save_one_kern`; `deregister` (`:308`) → `store.delete_one_kern`.
- [ ] **Step 3:** `persist::load_dir` → `Store::open` + `load_all_kerns` + `from_saved_with_mode`; `persist::save_all` → `store.save_all_kerns`. Keep `merged_root` (still needed for the root overlay).
- [ ] **Step 4:** `cargo build` + `cargo test base::` — green. Fix fallout.
- [ ] **Step 5:** Commit `feat(store): GraphGnn persists through redb`.

## Task 8: Route tick + cold callers

**Files:** `src/tick/tasks.rs`, `src/tick/stigmergy.rs`, `src/mcp/tools_query.rs`, `src/commands/graph_ops.rs`, `src/commands/reembed.rs`

- [ ] **Step 1:** `do_persist` → `store.save_one_kern` (root via `merged_root`).
- [ ] **Step 2:** stigmergy spill/compact → `store.cold_spill` (+ cap; `maybe_compact` becomes a no-op/removed since the table self-caps).
- [ ] **Step 3:** query/graph_ops/reembed cold reads → `store.cold_search`/`cold_get`/`cold scan`.
- [ ] **Step 4:** `cargo build` + `cargo test` — green.
- [ ] **Step 5:** Commit `feat(store): route tick + cold callers through redb`.

## Task 9: `kern migrate` + delete legacy

**Files:** Create `src/base/migrate.rs`; modify `src/commands.rs`, delete `src/base/cold.rs`, prune `src/base/persist.rs`.

- [ ] **Step 1:** Move the legacy file-shard *reader* (`load_dir`/`load_kern`/`backfill_created_at`/`migrate_root_id` usage) into `migrate.rs` as `read_legacy_dir(dir) -> (kerns, network_id)`; add `migrate_dir(old_dir)` that writes a `Store` next to it. Test: write a legacy `.kern` set via retained writer in the test, `migrate_dir`, open store, assert kerns present.
- [ ] **Step 2:** Add `Migrate { path: Option<String> }` subcommand → calls `migrate_dir`, prints counts, leaves old dir in place (user deletes).
- [ ] **Step 3:** Delete `cold.rs`; remove dead file-shard writers (`save_kern`/`load_kern`/`delete_kern`/`save_all` file versions/`sweep_stale_tmp`/`atomic_write`) now that nothing calls them.
- [ ] **Step 4:** `cargo build` (deny warnings — no dead code) + `cargo test` (full) — green.
- [ ] **Step 5:** Commit `feat(store): kern migrate + remove legacy file-shard tier`.

## Task 10: Verify + persona review

- [ ] **Step 1:** `cargo test` full suite green; record pass count.
- [ ] **Step 2:** Manual: point a daemon at a `data_dir`, ingest, restart, confirm recall + `kern.redb` present, no `.kern` files written.
- [ ] **Step 3:** Run the kern `personas` panel (storage/durability + Rust) on the diff; address top findings.
- [ ] **Step 4:** Commit any fixes.

---

## Self-review checks
- **Phase 5 (deferred, not in this plan):** entity-granular `ENTITY`/`REASON`/`META` tables + per-entity persist-dirty to kill write-amplification. Tracked here so it isn't lost.
- **Repo law:** redb is an embedded library (in-process, no network hop, no fallback) — a storage primitive like bincode, not a "pluggable backend"; complies with the no-pluggable-backend law. `// SHARED-CANDIDATE` note at top of `store.rs` (kept kern-local for now by decision).
- **No compat:** legacy format read only via the fenced `migrate` path; all legacy writers deleted.
- **bincode positional:** unchanged (per user — tagged codec is a separate mechanics change); migration reads legacy under the current struct layout so no reorder hazard during the swap.
