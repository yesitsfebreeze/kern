# src/log/src/lib.rs — commentary

- `log`: the sink branch moves `message` into the `Entry` without cloning; an earlier version cloned unconditionally just to keep a copy for the eprintln fallback, which the else branch borrows instead. Don't reintroduce the clone.
Second-pass migration:
- `klog!` doc trimmed to the anti-shadowing rule. Moved here: the level-specific `info!` / `warn!` / `error!` macros are the usual entry points; reach for `klog!` only when the level is dynamic.
