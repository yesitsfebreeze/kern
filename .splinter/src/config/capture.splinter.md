# src/config/capture.rs — commentary

- `digest_min_trust` (why the gate exists): the digest is a persistent re-injection surface — a poisoned/low-trust claim that lands there is replayed into every future session. The gate quarantines low-trust and repeatedly-contradicted claims (whose grown `conf_beta` drags `conf_mean` down) out of that surface. The `f64` type also avoids silent f32→f64 rounding against `build_digest`'s threshold.
- `digest_token_budget` (why tight): context rot — attention degrades with length, so a tight budget beats a long dump.
Memory capture + recall, ON by default. Field semantics:
- enabled: master switch for the intake + digest tasks.
- dir: intake directory (cwd-relative) deltas are written into.
- poll_secs: how often the intake is drained.
- digest_path: output path (cwd-relative) for the recall digest.
- digest_secs: how often the digest is regenerated.
- digest_k: max thoughts included in the digest (item-count cap).
- digest_min_trust: trust floor (conf_mean) a claim must clear to re-enter the digest; 0.0 disables the gate.
- digest_token_budget: approximate token budget for the digest body (best-first by heat x confidence); 0 disables the token cap (digest_k still caps count).
- done_retention_secs: retention window for archived deltas under <dir>/done/ — a transient audit trail swept each drain cycle (the graph itself is durable).
cwd-relative hazard kept in source: dir and digest_path must stay cwd-relative and independent of data_dir or the cwd-relative contract breaks. validate() rejects zero poll/digest intervals (they busy-loop the tasks) only when enabled.
