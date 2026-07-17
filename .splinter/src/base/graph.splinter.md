# src/base/graph.rs — commentary

- `consolidate_disk_index`: the inline stop-the-world build matches the tick's
  serialized model (every task holds the write lock), and it is gated to fire at
  most hourly and only past `DISK_CONSOLIDATE_MIN_DELTA`, so it is rare — but on a
  very large corpus it stalls everything. Planned follow-up: a non-blocking
  two-phase consolidate (build outside the lock against a transitional
  write-routing delta, swap under a brief lock) — see the backlog in
  docs/superpowers/plans/2026-06-12-diskann-wiring.md.
- `gc_empty_kerns`: why the liveness reap replaced the old leaf-first
  (`children.is_empty()`) reap — the unnamed-child spawn runaway produced a
  *cyclic* forest of empty kerns where every node has children and no childless
  leaf ever exists, so a leaf-first pass could never start and left hundreds of
  thousands of empty shards on disk. Motivation for reaping at all: the runaway
  fragments the graph to `max_kerns` near-empty kerns, and every retrieval seed,
  tick `enqueue_all`, and `/graph` render is O(loaded kerns), so the bloat is a
  flat tax on latency. The dangling-child scrub is one linear pass keyed on a
  membership set — O(total children), not O(victims x children), which would
  explode when the root holds hundreds of thousands of dead child refs.
- `deregister`: the explicit per-kern on-disk delete is a holdover from the
  file-shard tier (its `load_dir` read every `*.kern` as live); the store
  reconciles on `save_all` anyway, but the immediate delete keeps disk and memory
  in step.

Second-pass migration (comment -> note):
- `rebuild_index` spill decision, in full: above `disk_threshold` AND with a
  non-empty `data_dir`, entity vectors spill to a DiskANN/Vamana snapshot under
  `<data_dir>/diskann/entity`; otherwise the index is in-RAM HNSW. A build OR open
  failure logs a warning and falls back to in-RAM, so a disk error degrades the
  index rather than leaving the graph unsearchable. `gnn_entity_idx` and
  `reason_idx` never spill — only the entity index does.
- `consolidate_disk_index` COST, spelled out: the inline Vamana build runs under
  the graph WRITE lock, a stop-the-world pause scaling with resident entity count.
  A build failure falls back to a full in-RAM `rebuild_index`. (The planned
  non-blocking two-phase consolidate is in the bullet above.)
- `gc_empty_kerns` contract detail: everything not reachable by upward liveness is
  reaped even when it still has children, and dangling child refs are then
  scrubbed; the function returns the reaped count. `gc_empty_kerns_counted` wraps
  it as `(before, reaped, after)` — the shape both the startup reap and the offline
  `gc` command log.
- `resident_searchable_entity_count` deliberately mirrors `index_kern_into`'s
  filter (non-Superseded AND vector-bearing) — the two must agree or the spill
  decision is made on a count the index does not actually hold.
- `collect_entity_items` sorts via `BTreeMap` for id order specifically so the
  seeded Vamana build is reproducible (the same determinism law as the id-sorted
  HNSW insert that stays inline).
- Test contracts moved out of comments: the disk-vs-RAM mirror test uses distinct
  per-dim frequencies so the nearest-neighbour structure is unambiguous despite
  in-RAM int8 quant noise vs raw f32 on disk. The consolidate test mirrors the live
  path by writing to the source of truth AND the index/delta. The GC cycle test's
  setup — a named anchor and an entity-bearing kern under root, plus a root whose
  child list references all four children — mirrors the real on-disk root so the
  dangling-ref scrub is actually exercised.
