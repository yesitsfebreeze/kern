# src/base/vector_backend.rs — commentary

- `VectorBackend::Resident`: the historical behavior — right for a resident-sized set.
- `VectorBackend::Disk`: the path that keeps a huge resident set off the heap; wiring plan at `docs/superpowers/plans/2026-06-12-diskann-wiring.md`. The method-mirroring design means `base::search` call sites never learn which backend they hit.
Second-pass migration (2026-07-17):
- `Disk` invariant spelled out (inline doc keeps the compressed form): `insert(id, v)` writes v to delta AND tombstones id — the tombstone shadows any now-stale snapshot copy; `delete(id)` removes from delta AND tombstones; search reads snapshot MINUS tombstones unioned with delta, so no id is ever served from both halves (no double-count, no stale vector). `union_rank`'s higher-score dedupe is only a defensive backstop for this.
- `disk()`: starts with an empty delta and no tombstones; the snapshot is built by `GraphGnn::build_entity_disk_index`; the delta HNSW is (m=16, ef_construction=200).
- `resident()` mirrors `HnswIndex::with_mode` and is the default for a new/rebuilt index; the Resident/Disk routing decision lives entirely in `GraphGnn::rebuild_index` (moved from module doc).
- `pending_delta_len`: a large delta means the in-RAM overlay has grown and the snapshot should be rebuilt (consolidation trigger).
- `union_rank` ordering: score desc, id-asc tiebreak — same convention as `base::search::merge_hits`.
- `insert`/`delete` are plain upserts/no-ops-if-absent mirroring `HnswIndex` (docs removed as labels).
