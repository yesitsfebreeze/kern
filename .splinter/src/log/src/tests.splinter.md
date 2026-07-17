# splinter: src/log/src/tests.rs

Second-pass migration:
- `global_sink_installs_once_then_routes_log` is the only test touching the process-global SINK OnceLock; every other test uses a local `Sink::new()`. That keeps the assertions deterministic in the shared test process: no other test can have set SINK first, so the pre-install state is observably the eprintln-fallback branch.
- `concurrent_pushes_are_thread_safe_and_stay_capped`: the assertion is that 8 threads x 500 pushes neither panic/deadlock nor exceed MAX_ENTRIES.
