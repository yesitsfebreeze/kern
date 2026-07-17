# src/config/graph.rs — commentary

- `max_kerns` evict/persist bug (why the default is uncapped): the runaway fragmentation was observed live — 1024 kerns / 13 entities on a real graph with a finite cap enabled. Bug tracked in kern memory — query "finite max_kerns cap evict/persist bug".
disk_threshold: entity count above which rebuild_index spills the vector index to a disk-resident DiskANN snapshot; KERN_CAP_DISABLED (usize::MAX) = never spill.

max_kerns hazard (kept in source): do NOT set a finite cap. Eviction drops unpersisted `children` pushes, re-spawning a child every tick until the graph fragments. The underlying evict/persist bug must be fixed before any finite cap is safe.
