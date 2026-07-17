# src/tick/pulse.rs — commentary

- `LAST_GC_AT_SECS` / `LAST_CONSOLIDATE_AT_SECS`: the timing decision lives in the pure `should_run_gc` / `should_consolidate` (taking now/last/interval as args, unit-tested directly); the statics are only thin single-flight latches. Tests exercise the cadence logic via the pure fns and never touch the statics, keeping them parallel-safe.

## Second-pass migration:
- `should_run_gc` clock guards (moved from inline): `now_secs == 0` means callers couldn't read the clock → refuse; `last_secs > now_secs` is a regressed clock (skew) → refuse, to avoid amplifying time travel. `should_consolidate` shares these by delegating.
- Below-threshold pulses are no-ops by contract, so they also skip GC / reembed / consolidate fan-out; the next above-threshold pulse triggers the sweep.
