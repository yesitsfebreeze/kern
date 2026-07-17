# splinter: src/watcher/tests/watcher_tests.rs

Second-pass migration:
- Module doc detail: generous timeouts + tolerance for platform variation in *which* events fire (Windows often reports a Modified before a Created on the first write); asserts are on observed kinds across a window, never exact ordering.
- `collect_until` rationale: early-exiting the instant the predicate holds means fast machines don't pay a fixed worst-case sleep while slow CI still gets the whole budget; it replaces the `sleep(fixed); collect_events(budget)` pattern for presence assertions. `collect_events` (full-budget drain) remains required for negative assertions.
- `debounce_collapses_rapid_modifies_to_one_event`: on Windows a write+close burst produces several raw notify events per syscall — exactly the burst the 50 ms debounce coalesces.
