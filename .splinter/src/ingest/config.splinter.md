# src/ingest/config.rs — commentary

Second-pass migration (from source comments):

- `Config`: the runtime form and the serde `crate::config::IngestConfig` describe the same knobs at two layers. Both source the shared `INGEST_*` constants in `base::constants`, so the defaults cannot drift; `runtime_and_serde_ingest_defaults_agree` is the guard against a future edit re-introducing a divergent literal in one layer. The runtime form exists separately because it carries `ttl_secs`.
- `dedup_threshold`: a new vector whose nearest neighbour scores at or above the floor is treated as a duplicate and merged instead of inserted. Higher → fewer merges (a stricter notion of "same thought").
- `valid_from`: set from a distilled `valid_from` hint ("since March"), stamped onto the ingested entity's `valid_from`.
- `validate`: rejects an out-of-range knob at construction time rather than letting it surface as silently-wrong behaviour deep in ingest.
