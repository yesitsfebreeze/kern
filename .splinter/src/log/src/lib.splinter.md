# src/log/src/lib.rs — commentary

- `log`: the sink branch moves `message` into the `Entry` without cloning; an earlier version cloned unconditionally just to keep a copy for the eprintln fallback, which the else branch borrows instead. Don't reintroduce the clone.
Second-pass migration:
- `klog!` doc trimmed to the anti-shadowing rule. Moved here: the level-specific `info!` / `warn!` / `error!` macros are the usual entry points; reach for `klog!` only when the level is dynamic.
## Design context (moved from source doc comments)

- `klog!` logs at an explicit `Level`. Kept in source: it is named `klog!` (not `log!`) so it never shadows `log::log!` in downstream code importing both crates.
