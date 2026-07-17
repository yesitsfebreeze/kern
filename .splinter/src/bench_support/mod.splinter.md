# src/bench_support/mod.rs — commentary

Module map (moved from the `//!` doc):
- `locomo` / `locomo_run`: live LoCoMo conversational-memory eval — load the dataset, ingest each dialogue through the real `Worker`, answer the QA probes, aggregate per-category quality + latency into an `EvalReport`.
- `trace`: replayable retrieval-trace JSON (corpus docs + queries + the expected ids each query should recall).
- `build`: construct a graph from a trace's documents for the harness.
- `replay`: run a trace's queries against a built graph.
- `sweep`: sweep retrieval parameters over a trace, emit CSV.
- `ndcg`: NDCG@k scoring of ranked results against expected ids.
- `embed`: deterministic stub embedder so trace replays are reproducible without a live embedding model.
# src/bench_support/mod.rs — commentary (migrated from source doc comments)

- This module is benchmark + evaluation scaffolding for kern's retrieval stack. It is bench/eval ONLY — NOT part of the production daemon path.
