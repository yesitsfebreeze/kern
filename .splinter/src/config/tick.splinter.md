# splinter: src/config/tick.rs

Second-pass migration:
- `TickConfig` is the serde view of `[tick]` in `kern.toml`; defaults come from the `TICK_*` constants in `base::constants` so the baseline lives in one place.
- `max_cluster_sample`: clustering a kern drives auto-naming and child-spawn. The cap bounds clustering cost on large kerns — above it the pass samples rather than reading every entity.
- `queue_capacity`: bounded capacity of the maintenance-tick task queue, sizing how much pending tick work may queue before backpressure.
- `interval_secs`: each tick does heat decay + stigmergy GC via `pulse` and re-enqueues clustering. `0` disables the driver, leaving compaction event-driven only.
