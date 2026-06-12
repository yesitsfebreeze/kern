# Condensation Forest — sub-DB storage substrate (design)

**Status:** design / approved-for-planning. Supersedes the deferred "Phase 5" note
in `docs/superpowers/plans/2026-06-10-redb-store.md`.

**One line:** Make storage mirror the graph: the single LMDB env becomes a
**forest of linked env files** — cold, vector-cohesive kern subtrees **condense**
into their own sub-DB; any env that grows too big **bisects by vector centroid**;
the hot env stays small. This realises the documented vision
(`2026-06-04-kern-unified-memory-design.md`: *"Condensation — cold clusters detach
to sub-DBs, lazy-link on query. Hot graph stays small."*) and structurally
eliminates the `MDB_MAP_FULL` bloat that motivated it.

---

## Problem & evidence

The graph persists into one LMDB env (`src/base/store.rs`, `MAP_SIZE = 4 GiB`).
`save_all_kerns` rewrites **every** kern row in a single copy-on-write txn on each
save (periodic loop, shutdown, startup reap). LMDB never returns freed pages to
the OS and cannot reuse freed pages while any read txn (CLI / recall hook / MCP /
daemon) pins the old snapshot — and nothing ever calls `clear_stale_readers()` or
compacts. The historical empty-kern spawn-runaway drove the file to the 4 GiB cap;
it has stayed there since.

Measured on the live data dir (2026-06-12):

| Metric | Value |
|---|---|
| `data.mdb` on disk | 4096 MB (exactly `MAP_SIZE`, fully materialised) |
| Live graph rebuilt via `kern compress` | **~53 MB (int8) / ~80 MB (none)** |
| Dead freelist pages | **~98% of the file** |

A single `save_all` (which COWs the whole ~53 MB graph, transiently needing ~2×
pages while readers pin the old version) can no longer find room → `MDB_MAP_FULL`,
re-fired on every save attempt during shutdown. The bug is the **absence of the
condensation mechanism the architecture already assumes.**

## Goals

- **G1 — Bounded envs.** No single LMDB env can grow unbounded; each stays small
  enough that compaction is cheap and `MDB_MAP_FULL` cannot recur.
- **G2 — Condensation.** Cold, vector-cohesive kern subtrees detach into their own
  sub-DB env; the hot working set stays small (load time + RSS track the *hot*
  graph, not the whole corpus).
- **G3 — Lazy-link.** A detached subtree remains reachable by id (routing /
  traversal) and by vector (retrieval), paging back in on demand.
- **G4 — Size valve.** An env over a size cap force-bisects by vector centroid into
  two linked sub-DBs, even when hot.
- **G5 — Crash safety.** No detach/bisect/page-in window can lose or duplicate a
  kern after recovery.

## Non-goals

- Not implementing DiskANN in this spec (it is the later in-segment search upgrade,
  `diskann-disk-index.md`; tracked as Phase 4).
- Not changing the in-memory `Kern`/`Entity` model, the codec
  (`[FORMAT_V1] ++ zstd(bincode)`), or the int8-on-disk vector projection.
- Not adding any legacy/compat path (repo law). Version stays `1.0.0`.

---

## Architecture

```
                .kern/
                ├── data/                 ← HOT env (working set)
                │   ├── data.mdb          ·  kern / cold / meta  (as today)
                │   └── manifest (DB)     ·  one row per detached segment  ← the links
                └── sub/
                    ├── <seg_a>/data.mdb  ← SEGMENT env (detached subtree A)
                    ├── <seg_b>/data.mdb  ← SEGMENT env (detached subtree B)
                    └── …

   hot ANN index also holds one "summary node" per segment (centroid → seg id)
   so retrieval surfaces a cold segment as a candidate without loading it.
```

- **Hot env** — `.kern/data`, unchanged tables (`kern`/`cold`/`meta`) **plus** a new
  `manifest` named DB. Holds only the warm working set + the manifest stubs.
- **Segment env** — `.kern/sub/<seg_id>/`, a full independent LMDB env (own
  `data.mdb`, own lock, own freelist, own 4 GiB cap) containing exactly one
  detached subtree's kerns/entities/reasons, written with the *same* `StoredKern`
  codec. Because it is written fresh from the live set, it is born compact.
- **Manifest row** (`ManifestEntry`, the link) — per segment:
  `{ seg_id, root_kern_id, anchor_vec, inner_radius, outer_radius, summary_vecs:
  Vec<StoredVec>, live_bytes, kern_count, entity_count, heat, last_access_ms,
  path }`.
- **Hot segment index** — each segment's `summary_vecs` are inserted into the hot
  index as pointer-nodes (`seg:<id>`), resolved to a `ManifestEntry`, not an
  `Entity`. This is the by-vector discovery seam.

### Two reach paths into a detached subtree (both extend existing primitives)

1. **By-id** (routing, traversal, `get(id)`): a lookup that resolves to a manifest
   stub triggers **page-in** of that segment env (extends the unloaded-set
   auto-reload, architecture.md:160).
2. **By-vector** (retrieval): `seed` search returns a `seg:<id>` summary node in
   top-k → page-in that segment → search within it → merge (extends the cold-tier
   `< k` fill, memory-bank.md:104–109).

---

## Components

### 1. `Forest` (new — `src/base/forest.rs`)
Owns the hot `Store` plus an LRU of **open segment env handles** (bounded by
`[graph] max_open_segments`, default 8). One responsibility: resolve a kern id or a
query vector to the right env, opening/closing segment envs under the cap.

- `manifest_put/get/delete/scan` over the hot env's `manifest` DB.
- `open_segment(seg_id) -> Arc<Store>` — LRU; on eviction of a *clean* (not written
  since page-in) handle, just drop it; a *dirty* handle is flushed first.
- `clear_stale_readers()` fan-out across the hot env + all open segment envs
  (called on open and each maintenance tick — the reader-reaping the daemon lacks).

### 2. Detach — `do_condense` (new tick task — `src/tick/condense.rs`)
Heat-driven. A subtree rooted at a **named** kern qualifies when:
`aggregate_heat < condense_heat_floor` AND `age > condense_min_age` AND it is a
cohesive named anchor (not root, not a configured hot anchor). Steps (ordered for
crash safety):

1. Serialise the subtree (root + descendants + their entities/reasons) into a fresh
   segment env via `save_all_kerns` (born compact), **fsync**.
2. Compute `summary_vecs` — the subtree's mean entity vector (Phase 2: a single
   centroid; refinement: ≤4 k-means centroids for multimodal subtrees).
3. **Commit the manifest row** (the linearisation point).
4. Prune the subtree's rows from the hot env, drop its kerns from the in-memory
   graph, and insert the `seg:<id>` summary node(s) into the hot index.

### 3. Page-in — `Forest::resident(seg_id)`
Open the segment env (LRU), `load_all_kerns`, splice the subtree back into the hot
graph + indexes, mark resident. On cooldown (heat low again) or LRU eviction it
**re-detaches**: if unchanged since page-in, just drop from memory and close the
handle (the env file is already current — no rewrite); if changed, re-serialise
first, then update `live_bytes/heat/summary`.

### 4. Size valve — `maybe_bisect(env)` (maintenance tick)
For the hot env and every open segment env, read `non_free_pages_size()` /
`real_disk_size()`. If `real_disk_size > segment_max_bytes` (default 256 MiB):
2-means over the env's kern `anchor_vec`s → two kern partitions → write two fresh
segment envs + two manifest rows → delete the old env + old manifest row + old
summary, insert two new summaries. (For the **hot** env, the larger/colder half
detaches as a segment until the hot env is under cap.)

### 5. Phase 0 — compaction + reader-check (`Store` + out-of-band task)
Independent of the forest; ships first and fixes the live bug.

- `Store::open` and the maintenance tick call `env.clear_stale_readers()`.
- An out-of-band **compaction task** (mirrors the journal compactor's drain pattern,
  `src/ingest/compactor.rs`): when `real_disk_size > compact_floor_bytes` AND
  `real_disk_size / non_free_pages_size > compact_bloat_ratio` (default 3.0), run
  `env.copy_to_file(tmp, CompactionOption::Enabled)`, then atomically swap the dir
  at a safe point (under the graph write lock, no in-flight write txn) and reopen.
  `copy_to_file` snapshots a live env, so the copy runs without blocking writers;
  only the swap is brief.

All of `clear_stale_readers`, `copy_to_file`, `CompactionOption`,
`non_free_pages_size`, `real_disk_size` are confirmed present in the locked
**heed 0.20.5**.

---

## Data flow

- **Detach (condense):** tick → `do_condense` qualifies a cold subtree → segment
  env written+fsync → manifest committed → hot rows pruned + summary indexed.
- **Retrieve (by-vector):** query embed → `seed` over hot index → a `seg:<id>`
  summary ranks in top-k → `Forest::resident(seg)` pages it in → search within →
  RRF-merge with hot hits → (segment cools → re-detach).
- **Route/traverse (by-id):** `route_entity` / beam `expand` / `get(id)` hits a
  manifest stub → page-in → continue. Ingest into a detached region pages it in,
  accepts, and lets it re-detach on cooldown.
- **Bisect (size):** maintenance sees an env over cap → 2-means split → two
  segments replace one.
- **Compact (Phase 0):** bloat ratio crossed → `copy_to_file(Enabled)` → atomic
  swap → reopen.

## Crash safety

- **Manifest commit is the linearisation point.** Segment env is written+fsync
  *before* the manifest row; hot rows are pruned *after*. Therefore:
  - Crash before manifest commit → segment env is an **orphan**; the subtree still
    lives in the hot env (no loss). Load-time sweep removes orphan `sub/<id>` dirs
    with no manifest row.
  - Crash after manifest commit, before hot prune → subtree exists in **both**;
    load-time reconcile makes **manifest win** and prunes the hot duplicates by id
    (idempotent, like today's `save_all_kerns` orphan prune).
- **Bisect** writes both new envs + manifest rows before deleting the old env/row,
  so a crash mid-bisect leaves the old segment authoritative (new envs are orphans,
  swept).
- **Compaction** swaps via atomic rename with the old dir retained as `*.bak` until
  the reopened env verifies, so a crash leaves a complete old or new dir, never a
  torn one.

## Config (`[graph]`)

| Key | Default | Meaning |
|---|---|---|
| `segment_max_bytes` | `256 MiB` | size valve: env over this bisects |
| `max_open_segments` | `8` | LRU cap on concurrently open segment envs |
| `condense_heat_floor` | `0.05` | subtree aggregate heat below this may detach |
| `condense_min_age_secs` | `7d` | minimum subtree age before detach |
| `compact_floor_bytes` | `64 MiB` | don't compact envs smaller than this |
| `compact_bloat_ratio` | `3.0` | compact when `disk_size / live_size` exceeds this |
| `hot_anchors` | `[]` | named anchors pinned resident (never detach) |

---

## Phasing (each independently shippable + persona-reviewed)

- **Phase 0 — reader-check + threshold compaction.** Unblocks the live bug. Lowest
  risk, no forest. *Recovery for the current 4 GiB file: `kern compress … --mode
  int8` into a fresh dir and swap during a controlled daemon restart.*
- **Phase 1 — manifest DB + segment env read/write + by-id page-in.** The forest
  exists; `Forest` resolves ids across hot + segment envs; LRU of open envs.
- **Phase 2 — `do_condense` + summary nodes in hot index (by-vector page-in).**
  Condensation runs; hot graph stays small.
- **Phase 3 — size valve (2-means bisection).** "Split in half sorted by vectors."
- **Phase 4 — DiskANN within large segments (later, separate spec).** Million-scale
  in-segment recall.

## Testing

- **Phase 0:** unit — `clear_stale_readers` invoked on open; compaction triggers
  only above floor+ratio; `copy_to_file` output is smaller and round-trips
  (`load_all_kerns` equal). Integration — synthesise a bloated env (many
  put/overwrite cycles with a held reader), assert post-compaction `real_disk_size`
  drops and data is intact.
- **Phase 1:** `manifest` round-trip; `Forest::resident` pages a segment in and a
  `get(id)` across the boundary returns the kern; LRU eviction closes a clean handle
  without data loss; orphan-segment sweep on load.
- **Phase 2:** a cold cohesive subtree detaches (hot row count drops, segment env
  appears, manifest row written, summary node present); a query whose vector matches
  the summary pages the segment in and returns its entities; crash-injection between
  manifest commit and hot prune → reconcile leaves exactly one copy.
- **Phase 3:** an over-cap env bisects into two envs whose kern sets partition the
  original by nearest centroid; recall over the union is preserved (extend
  `benches/` recall harness).
- **Crash matrix:** kill points before/after each fsync/commit in detach + bisect +
  compaction; assert no loss, no duplication after reload.

## Repo-law compliance

- **No compat.** No legacy reader; the forest is the only post-Phase-1 layout. A
  pre-forest single env simply has an empty `manifest` DB and behaves as today —
  this is a clean superset, not a migration shim.
- **Version stays `1.0.0`.**
- **bincode is positional.** `ManifestEntry` and any `summary_vecs` use the
  always-present-field discipline of `StoredVec`/`StoredKern` (no
  `skip_serializing_if`). Adding a field later = new struct/version byte, never a
  reorder. Flagged for every `#[derive(Serialize/Deserialize)]` touched.
- **Duplicate check.** `Forest` is new; it composes `Store` (does not reimplement
  the codec or KV). The cold tier stays in the hot env. No duplicate storage path.
- **shared/ check.** `Store` already carries a `// SHARED-CANDIDATE` note; `Forest`
  is kern-specific orchestration (depends on the kern graph/anchor model), so it
  stays kern-local. Re-evaluate lifting the *env-forest + compaction* primitive into
  `shared/` if relay/agnt need a self-bounding embedded store.
```
