# src/bench_support/latency.rs — commentary

- `measure_throughput`: exercises the read-only graph's concurrent-read scaling — the same path the MCP server and recall hooks share.

Second-pass migration (from the `//!` and item docs):
- Positioning: this is the speed complement to `replay`'s recall/NDCG quality scoring — `measure_latency` (single-reader percentiles) and `measure_throughput` (concurrent-reader qps), both over the LLM-free graph retrieval path. The "A/B over a fixed trace, not absolute SLAs" framing is kept inline in compressed form; the "not yet a Qdrant baseline" caveat lives here.
- `measure_latency`: applies the same `filter_kind` the recall harness uses, so a filtered run measures the filtered traversal's cost. The sub-ms graph-only path is what is timed (LLM/embedder hooks are `None` — that trap stays inline).
- `measure_throughput`: honest concurrent-read scaling — a `RwLock` write would serialize, and there is none on this path.
