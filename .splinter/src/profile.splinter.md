# src/profile.rs — commentary

- `profile_block!`: the `prof.checkpoint("body")` call fixes an old regression — the previous macro version finished the Profiler with zero checkpoints (and held a dead `start` Instant that was never read), so the logged profile carried no stage timing at all. Guarded by `profile_block_macro_returns_block_value_and_expands_clean`.

Second-pass migration:
- `render_timeline`: doc compressed to the scaling contract. Detail moved here — stage segments cycle the `FILLS` characters (`█▓▒░`) so a bar's composition stays visible, and the per-stage numbers follow the bar in parens. Bars are scaled against the slowest profile in the set; an empty set or an all-zero total renders "".
- The min-1-cell floor stays inline (compressed): rounding alone drops a small-but-nonzero stage to a 0-width, invisible bar, while a genuinely zero stage must stay empty. Guarded by `render_timeline_tiny_nonzero_stage_gets_at_least_one_cell`, whose fixture is 0.4/100*20 = 0.08 → `round()` = 0 cells without the floor; with it, `FILLS[1]` = '▓' must appear.
- Test narration duplicating assert messages was deleted from `profiler_records_checkpoints` and `render_timeline_scales_and_lists_stages`; the `profile_block!` regression rationale is already recorded above in this note, so its test comment was dropped too.
## Design context (moved from source doc comments)

- Module: query-latency profiler (`kern profile`) — labelled `Checkpoint`s split a recall into graph-engine vs LLM (HyDE / answer / distill) stages.
- `render_timeline`: renders profiles as an aligned ASCII timeline; bars are scaled to the slowest total.
- `profile_block!` macro instruments a code block with timing. Usage: `profile_block!("name", { /* code */ })`.
- Test `render_timeline_tiny_nonzero_stage_gets_at_least_one_cell`: `tiny` is 0.4/100*20 = 0.08 -> `round()` = 0 cells without the min-1 floor; with it, the stage (FILLS[1] = '▓') must still appear once. (The min-1-floor rationale is kept in source at the render site.)
