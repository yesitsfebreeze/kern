# src/config/watcher.rs — commentary

- `WatcherConfig`: the filesystem watcher landed as Slice O (file changes flow into kern as `Document` entities through `watcher::IngestSink`).

Second-pass migration:
- Opt-in shape, previously a doc-comment example:
  ```toml
  [watcher]
  enabled = true
  roots = ["./src", "./docs"]
  ```
- `effective_roots`: returns an empty vec when disabled so callers can treat "nothing to watch" uniformly instead of branching on `enabled` themselves. `cwd` is a parameter rather than read from the process so the "empty roots defaults to cwd" rule lives in exactly one place and stays unit-testable.
[watcher] ingests filesystem changes as Document entities, OFF by default. enabled is the master switch (false keeps it dormant even with roots set). effective_roots returns configured roots, else cwd when enabled, else empty when disabled; cwd is injected (not read from the process) for testability.
