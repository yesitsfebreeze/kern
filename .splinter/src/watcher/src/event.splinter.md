# src/watcher/src/event.rs — commentary

- `WatchEvent`: derives `Hash` deliberately so events can be `HashMap`/`HashSet` keys (e.g. dedup) without downstream boilerplate.
- `WatchEvent::new`: the `path == to` override is centralised in the constructor so a caller can't accidentally emit a `Renamed` whose `path` points at the old location.
Second-pass migration:
- `WatchEvent` doc detail compressed: `path` is the canonical path the event concerns; for `Renamed` the payload also carries `from` while `path == to` (the new location). The `new` constructor doc now states only the trap: the `path` argument is overridden with `to` for `Renamed`.
