# src/config/graph.rs — commentary

- `max_kerns` evict/persist bug (why the default is uncapped): the runaway fragmentation was observed live — 1024 kerns / 13 entities on a real graph with a finite cap enabled. Bug tracked in kern memory — query "finite max_kerns cap evict/persist bug".