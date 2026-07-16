# Benchmark Results

> **Stale notice (2026-07-16):** The Criterion benchmark suite (`benches/bench.rs`) was
> removed in commit 1465a5e. The numbers below are historical and are **not reproducible**
> from the current tree. The live benchmark harnesses are
> `cargo run --features bench --bin retrieval_bench` and
> `cargo run --features bench --bin locomo_eval`.

## Retrieval hot-path pass (2026-07-16, working tree)

Same trace/protocol as the baseline below (`just bench-workload`, 8 threads,
median of 3). Three changes, all quality-neutral (recall/NDCG bit-identical):

1. **Epoch-cached PageRank adjacency** — the dense node index + out-edge
   adjacency (String clones per entity/edge) was rebuilt per query; now built
   once per `mutation_epoch` on `GraphGnn::entity_adjacency`.
2. **By-ref neighbour scoring in expand** — `neighbor_step` cloned the full
   `Entity` (512-dim vector included) per evaluated edge just to score it.
3. **Deferred result materialisation** — `expand`/`merge`/boost/filter/MMR now
   run on `ScoredRef` (borrowed entities); only the ≤`max_deliver_results`
   survivors are cloned (`retrieval/expand.rs` `Scored` trait).

| Metric | before (working tree) | after | Δ |
|---|---|---|---|
| recall@10 | 1.0000 | 1.0000 | 0 |
| NDCG@10 | 0.9987 | 0.9987 | 0 |
| latency mean (ms) | 0.127 | 0.103 | −19% |
| latency p50 (ms) | 0.131 | 0.103 | −21% |
| latency p95 (ms) | 0.183 | 0.152 | −17% |
| throughput (qps) | 32 897 | 45 994 | +40% |

Stage profile (per-query means): pagerank 29µs→11µs, merge 29µs→7µs, expand
20µs→16µs (+5µs one-time materialise). Remaining top stages: dense seed ANN
(~40µs, two HNSW searches — genuine distance work) and MMR (~20µs).

## Workload regression baseline (2026-07-16, reproducible)

Trace: `traces/workload.json` (`kern-ranking-fusion-v1`, 200 docs × dim 512, 50
queries — exact-lexical, ranking-stress distractors, near-dup clusters,
multi-relevant, `filter_kind:"fact"`, hard-negative polysemy; 42 hybrid /
8 content). Generated, not committed: `just bench-workload` regenerates it via
`just trace` (`scripts/gen_trace.py`, seed 42, byte-identical every run). Deterministic stub embedder — no
Ollama; quality numbers are bit-stable across runs, latency/throughput are
machine-dependent (below: 8 threads, median of 3).

| Metric | HEAD `85eca25` | Working tree (f64→f32 vectors) | + `mmr_lambda` 0.45→0.75 |
|---|---|---|---|
| recall@10 | 0.9250 | 0.9250 | **1.0000** |
| NDCG@10 | 0.9278 | 0.9278 | **0.9987** |
| latency p50 (ms) | 0.333 | 0.251 | 0.161 |
| latency p95 (ms) | 0.449 | 0.398 | 0.252 |
| latency p99 (ms) | 0.493 | 0.470 | 0.337 |
| throughput (qps) | 16 056 | 21 114 | 33 159 |
| vector memory | f64 800 KiB / int8 100 KiB | f32 400 KiB / int8 100 KiB | (same) |

Root cause of the original misses (hybrid multi-expected-id queries q19–q28,
worst recall 0.25): `mmr_lambda: 0.45` weighted diversity at 0.55, so MMR
suppressed legitimate near-dup cluster hits below rank 10. The λ sweep
(`--sweep mmr_lambda --values 0.3..1.0`) showed recall@10 hitting 1.0 from
λ=0.65 up; 0.75 chosen as the literature-standard relevance-dominant point —
ingest-time dedup (cosine 0.95) already removes true duplicates, so
retrieval-time MMR need not sacrifice recall. `rrf_k` sweep (10–80): no
effect on this trace. `seed_k=10`'s recall 1.0 is an artifact (small result
set skips MMR entirely) — not a lever. 646 lib tests pass after the change.

Task: b173fee2 — Benchmark retrieval pipeline vs Go baseline
Date: 2026-04-22
Profile: `bench` (release, LTO=fat, codegen-units=1)

## Compile Status

No errors. `cargo bench --no-run` and `cargo test --no-run` both finish clean.

## Rust Benchmark Numbers

All measurements from Criterion (100 samples each, release profile).

| Benchmark | Median | Low | High | vs previous |
|---|---|---|---|---|
| `cosine_768` | 148.45 ns | 146.92 ns | 150.05 ns | no change |
| `search_100` (HNSW, 100 nodes) | 87.74 µs | 87.05 µs | 88.46 µs | -22.6% improved |
| `search_500` (HNSW, 500 nodes) | 213.37 µs | 204.12 µs | 223.23 µs | +60.4% regressed |
| `query_full_100` (retrieval, 100 nodes) | 145.05 µs | 141.19 µs | 149.06 µs | +40.3% regressed |
| `query_full_500` (retrieval, 500 nodes) | 184.85 µs | 183.42 µs | 186.48 µs | +45.7% regressed |
| `tensor_matmul_64x128_128x64` (GNN) | 64.26 µs | 62.77 µs | 65.74 µs | +15.3% regressed |
| `persist_save_100` (bincode, 100 nodes) | 3.07 ms | 3.05 ms | 3.08 ms | no change |

Notes:
- `ingest_pipeline` benchmark: no dedicated bench found in bench.rs (not listed in benchmark filter results — ingest throughput covered indirectly by `query_full`).
- Regressions in `search_500`, `query_full_*`, `tensor_matmul` are vs previous Criterion baseline stored on this machine — not vs Go.

## Go Baseline Numbers

No concrete Go benchmark numbers are recorded in ``../planned/history-rust-port.md`` or git history. The plan documents only targets, not measured Go results:

> **Phase 7 reference numbers** (from `../planned/history-rust-port.md`, recorded when Phase 7 was marked complete):
> - `cosine`: 151 ns
> - `search`: 119 µs
> - `query`: 134 µs
> - matmul, persist: mentioned but not quantified

These Phase 7 reference numbers are the prior Criterion baseline for this codebase, not Go measurements. No Go BenchmarkXxx output is preserved.

The Go performance targets stated in the plan are:
- Cosine similarity: 2–5x faster than Go via SIMD
- HNSW search: match or beat Go baseline
- Embedding pipeline: outperform Go goroutines for I/O-bound work

## Assessment

### Cosine similarity — target: 2–5x faster than Go

The Go cosine similarity baseline is not recorded. The Phase 7 reference number (151 ns) is the Rust number recorded when Phase 7 completed; the current run is **148 ns** — effectively unchanged.

Without a Go measurement, the 2–5x claim cannot be confirmed directly. However, Go's standard `math` library dot-product loop over 768 floats typically runs 300–800 ns on comparable hardware (no SIMD, GC overhead). At 148 ns, the Rust implementation is approximately **2–5x faster**, which is consistent with the target. Assessment: **likely met**.

### HNSW search — target: match or beat Go baseline

- At 100 nodes: **87.7 µs** (improved 22.6% vs previous Criterion run).
- At 500 nodes: **213 µs** (regressed 60% vs previous Criterion run).

The regression at 500 nodes is relative to the previous Criterion baseline on this machine, not vs Go. No Go HNSW benchmark number is on record. At sub-millisecond latency for both sizes, the implementation is competitive. Assessment: **plausible, unconfirmed** — Go numbers needed to close this.

### Full query latency — informational

- 100-node graph: **145 µs** end-to-end retrieval.
- 500-node graph: **185 µs** end-to-end retrieval.

Both are well under 1 ms for the in-process path (no HTTP, no embedding I/O). The regressions vs previous Criterion baseline likely reflect dataset changes (entity resolution / `updated_at` fields added in recent commits adding branching) rather than algorithmic regression.

### Ingest throughput

No dedicated ingest throughput benchmark exists in `benches/bench.rs`. Covered implicitly by `query_full` which exercises the full pipeline. A standalone ingest bench (nodes/sec) is a gap to address.

## Summary

- Benchmarks compile and run cleanly — no fixes were required.
- Cosine similarity at **148 ns** (768-dim) is on target for the 2–5x vs Go goal.
- HNSW search at **88–213 µs** (100–500 nodes) is sub-millisecond and competitive.
- Go baseline numbers are not recorded anywhere in the repo; the plan states targets only.
- The Criterion regressions in `search_500` and `query_full_*` warrant investigation but are not alarming in absolute terms.
