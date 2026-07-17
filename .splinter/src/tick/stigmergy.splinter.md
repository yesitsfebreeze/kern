# src/tick/stigmergy.rs — commentary

Implements the loop promised in `docs/kern/stigmergy-self-improving.md`: "unused pheromone evaporates → thought cools → automatic garbage collection via `forget()`". The Fact/Document durable-kind immunity is per `docs/kern/safety-architecture.md`.

- `is_cold_victim`: split out from `run_gc`'s lock/store plumbing so the GC policy is unit-testable in isolation.
- `evict_victims`: split out from `run_gc` so the drop-iff-persisted invariant is unit-testable without a failing store.
- `run_gc`: the `kept > 0` warn exists because a persistently failing cold store would otherwise silently let hot memory grow with no signal that GC is stalled on a broken store.

## Second-pass migration:
- `is_cold_victim` policy (moved from inline): drop iff cold (`heat < COLD_HEAT_THRESHOLD`), stale (`now - last_touch > COLD_GC_AGE`), and non-durable. The staleness clock reads `accessed_at`, falling back to `created_at` for entities never queried since ingest — so old-but-never-accessed thoughts still become evictable. Neither timestamp → preserved; a future timestamp (clock skew) → not stale. Fact/Document are immune UNLESS superseded: a bi-temporally invalidated fact is history, not live knowledge, and loses immunity so it can spill to the cold tier (invalidated ≠ deleted). Losing immunity means "subject to GC", not "force-evicted" — a recently-touched superseded fact is still spared.
- `run_gc`: `cold_spill` self-caps the cold tier (drops oldest past COLD_MAX_ENTRIES), so no separate compaction pass is needed. The store handle is cloned out (ref-counted) so the graph keeps mutating under the single write guard.
