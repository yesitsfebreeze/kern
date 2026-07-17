# splinter: src/bench_support/stage_profile.rs


Second-pass migration (from the `//!` and `measure_stage_profile` docs):
- Positioning: the stage-level companion to `latency`'s whole-path percentiles. It runs the LLM-free graph phase through `crate::retrieval::answer::retrieve_profiled` and aggregates each stage's mean/p50/p95 (ms) plus its share of the total, so a config/index change can be attributed to the stage it moved.
- Stages are keyed by label and aggregated across all queries and iterations. Output follows first-seen stage order so the table reads seed → chains — that ordering invariant stays inline.
- `aggregates_stages_and_shares_sum_to_the_whole` asserts `share_sum` in `0.5..=1.5`: stage means sum to roughly the mean total because checkpoint gaps tile the total, so shares sum to ~1.0 barring the tiny inter-stage slack the Profiler leaves. The wide band is deliberate — it is a sanity check, not a precision assertion.
