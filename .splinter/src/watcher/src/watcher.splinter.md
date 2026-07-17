# src/watcher/src/watcher.rs — commentary

- `flush_due`: collects due keys into a Vec first because the borrow checker forbids removing from `pending` while iterating it; only the `PathBuf` keys are cloned (never the `Pending` values), so it stays cheap on a large pending set.
Second-pass migration:
- `FileWatcher` shutdown semantics (moved from struct doc): dropping the watcher stops the background coalescer — `_notify` drops first (field order, kept inline), closing the raw std channel, which makes the coalescer loop exit cleanly and flush pending events.
- `receiver()` exists so callers can plumb the receiver into `tokio_stream::wrappers::UnboundedReceiverStream` themselves.
- `translate` rationale (moved from doc): lone `From`/`To` rename halves become `Deleted`/`Created` because that matches what a user observes when rename endpoints straddle the watch root; `Modify(Name(Both))` is the debouncer-style rename carrying both endpoints.
- Test `translate_rename_both_with_wrong_arity_is_not_a_rename`: real filesystems can deliver a `Both` event with fewer than 2 paths; it must degrade to `Modified` — never a `Renamed`, never a panic.
- `Pending` coalescing (moved from struct doc): the coalescer keys pending events by path in a `HashMap`, so a later raw event for the same path overwrites the earlier un-emitted one — last-write-wins within the 50 ms debounce window. Each insert resets that path's deadline; only the newest event per path is ever emitted.
