# splinter: src/bin/retrieval_bench.rs


# src/bin/retrieval_bench.rs — commentary

CLI over `bench_support`: builds a graph from a `--trace` and either scores it (recall@10 / NDCG@10), sweeps one `SweepParam`, or runs one of the measurement legs (`--latency`, `--throughput`, `--mixed`, `--memory`, `--profile`, `--all`).

Second-pass migration:
- The `//!` doc's JSON trace example was deleted; it duplicated the one on `bench_support::trace::Trace`, and the schema now lives once in that module's splinter note. The `//!` points at `trace::Trace` instead.
- `docs` seed the graph; each `query` is scored against its `expected_ids` using its declared `mode`. `--mode <m>` restricts the run to queries declaring that mode — docs are untouched, so the graph is identical and only the scored query set narrows. That invariant stays inline because it is not obvious from the `retain` call.
- The `--values` up-front validation comment was deleted; the error strings are self-explanatory. NOTE: `--values` is validated twice — an emptiness check before `SweepParam::parse` and an `is_empty()` check on the parsed vector after. The second is close to unreachable (a non-empty `--values` of only commas parses to an empty vec, which is the one path that reaches it). Left as-is: this pass is comments-only.
- clap `///` arg docs are `--help` output and were left intact.
