# splinter: src/bin/retrieval_bench.rs


# src/bin/retrieval_bench.rs — commentary

CLI over `bench_support`: builds a graph from a `--trace` and either scores it (recall@10 / NDCG@10), sweeps one `SweepParam`, or runs one of the measurement legs (`--latency`, `--throughput`, `--mixed`, `--memory`, `--profile`, `--all`).

Second-pass migration:
- The `//!` doc's JSON trace example was deleted; it duplicated the one on `bench_support::trace::Trace`, and the schema now lives once in that module's splinter note. The `//!` points at `trace::Trace` instead.
- `docs` seed the graph; each `query` is scored against its `expected_ids` using its declared `mode`. `--mode <m>` restricts the run to queries declaring that mode — docs are untouched, so the graph is identical and only the scored query set narrows. That invariant stays inline because it is not obvious from the `retain` call.
- The `--values` up-front validation comment was deleted; the error strings are self-explanatory. NOTE: `--values` is validated twice — an emptiness check before `SweepParam::parse` and an `is_empty()` check on the parsed vector after. The second is close to unreachable (a non-empty `--values` of only commas parses to an empty vec, which is the one path that reaches it). Left as-is: this pass is comments-only.
- clap `///` arg docs are `--help` output and were left intact.
# src/bin/retrieval_bench.rs — commentary (migrated from CLI doc comments)

- Binary: replays a retrieval trace and reports NDCG@10, optionally sweeping one `SweepParam` over a list of values. Trace format: `trace::Trace`.
- `--mode` keeps only queries declaring that mode; docs are untouched so the graph is identical — only the scored query set narrows.
- CLI legs (help text removed from source): `--latency` graph-retrieval latency p50/p95/p99 over the trace (LLM-free; warmup + timed iters per query); `--throughput` retrieval qps under N concurrent reader threads (default: available parallelism; LLM-free); `--threads` reader-thread count; `--mixed` read/write/persist contention run (N readers on the locked query path + M writers doing accept() + one persist thread; reports read p50/p95/p99, read qps, write ops/s, and the worst single read stall); `--writers` writer threads for --mixed (default 2); `--secs` wall-clock seconds for --mixed (default 10); `--memory` vector-storage footprint (f32 vs int8); `--profile` per-stage timings (seed/fuse/expand/merge/boosts/mmr/chains) with mean/p50/p95 and share (LLM-free); `--all` one combined Tier-0 snapshot (corpus size, recall@10/NDCG@10, latency p50/p95/p99, throughput, vector memory in a single run).
