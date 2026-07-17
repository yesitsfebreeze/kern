# src/tick.rs — commentary

- `TickContext`: bundles the tick worker's long-lived deps so `start`/`process_task` take one context instead of eight positional args (and dropped their `too_many_arguments` allows). All hooks are cheap `Arc` clones; the configs are small value types.
- `spawn_child_clusters`: history of the one-distinct-child rule — `get_or_spawn_unnamed_child` reused the parent's first unnamed child, so every selected cluster collapsed into the SAME kern and `spawned_children` carried that id N times, enqueuing N duplicate Cluster/Persist tasks. One fresh child per cluster is what phase 5's per-child task enqueue assumes. Regression test: `spawn_child_clusters_creates_a_distinct_child_per_cluster`.

## Second-pass migration:
- `do_cluster` persist ordering: children are persisted BEFORE the parent because a spawned child lives only in RAM until its Persist runs, while Persist(parent) rewrites the parent row WITHOUT the migrated entities. Parent-first + crash erases those entities from disk entirely (child row never written); child-first + crash merely duplicates them in the stale parent row until the next persist. Guard: `do_cluster_persists_each_spawned_child_before_the_parent`.
- `select_spawn_clusters` unnamed gate rationale (moved from inline): a fresh child is by construction one cohesive cluster, and `do_cluster` spawns grandchildren (phase 2) before the Name task is enqueued (phase 5) — so an unnamed kern that may spawn descends one level per pass unboundedly (observed live: tens of thousands of empty unnamed kerns). An unnamed kern's empty `anchor_vec` means the `is_core_cluster` skip cannot protect it. Holding thoughts until Name succeeds is loss-free.
- Phase-number labels (1..5) removed from the inline flow; the phase-N doc comment on each helper is the remaining map.
## Design context (moved from source doc comments)

- Module: background tick scheduler — the autonomic loop. Clusters thoughts, spawns/evicts child kerns, GNN propagation, heat decay, cold-GC.
- `do_cluster` is decomposed into phases:
  - Phase 1 `select_spawn_clusters` — pure read: cluster the kern's thoughts, pick the spawn indices.
  - Phase 2 `spawn_child_clusters` — one unnamed child per selected cluster; members move out of the parent.
  - Phase 3 `collect_follow_up_jobs` — pure read; returns (un-enriched real edges, dangling Question edges).
  - Phase 4 `evict_empty_children` — reap empty AND unnamed children, reparenting strays; returns true if any evicted.
- Test `select_spawn_clusters_still_spawns_off_core_cluster_from_named_kern`: entities orthogonal to the anchor are off-core yet cohesive among themselves, so a named kern still spawns off them.
- Test `do_cluster_skips_gnn_when_no_structural_work`: one thought, no edges, no children -> no structural work of any kind.
- Kept in source (load-bearing hazards): persist children BEFORE the parent (parent-first + crash erases migrated entities from disk; child-first only briefly duplicates); no structural change -> prior gnn_vector still valid, skip GNN; UNNAMED KERNS NEVER SPAWN (else each pass descends one level unboundedly); one DISTINCT child per cluster (never `get_or_spawn_unnamed_child`, which reuses the first unnamed child and collapses every cluster into one kern).
