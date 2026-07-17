# src/watcher/src/lib.rs — commentary

Ignore matching reuses the `ignore` crate, already a dep of `shared/search` — no duplicated gitignore semantics.

Platform quirks behind the 50 ms debounce:
- Windows (`ReadDirectoryChangesW`) fires multiple events per logical edit (open-for-write, write, close, metadata); debounce coalesces them into one. Editors that swap-rename on save (vim, VS Code) appear as `Renamed { from, to }` when both endpoints are inside a watched root, otherwise as separate `Deleted` + `Created` — matches notify's documented behaviour, preserved intentionally.
- macOS FSEvents may coalesce server-side; debounce still applied for symmetry.
- Linux inotify fires one event per syscall; debounce mostly drops editor-induced bursts (write + chmod + close-write).
Second-pass migration:
- Module `//!` doc compressed to 2 lines. Moved here: the usage sketch — implement an `IngestSink`, then pump a `FileWatcher`'s coalesced events through `IngestPipeline::handle` into it; this is exactly how kern wires its MCP `ingest` call. Feature list: cross-platform recommended-watcher mode, 50 ms per-path debounce (drops intermediates), `.gitignore` + `.kernignore` honouring via the `ignore` crate.
