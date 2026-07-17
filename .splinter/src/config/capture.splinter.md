# src/config/capture.rs — commentary

- `digest_min_trust` (why the gate exists): the digest is a persistent re-injection surface — a poisoned/low-trust claim that lands there is replayed into every future session. The gate quarantines low-trust and repeatedly-contradicted claims (whose grown `conf_beta` drags `conf_mean` down) out of that surface. The `f64` type also avoids silent f32→f64 rounding against `build_digest`'s threshold.
- `digest_token_budget` (why tight): context rot — attention degrades with length, so a tight budget beats a long dump.