# src/lib.rs — commentary

Second-pass migration: the crate `//!` header was a five-bullet responsibility list (memory / retrieval / llm / ingest / rpc). It is now two lines; the per-module `///` one-liners on each `pub mod` already carry the same map and are the rustdoc surface, so the bullets were pure duplication. Kern is a daemon (`kern --daemon`) exposing its surface over MCP stdio and HTTP; responsibilities: memory = CRDT-replicated knowledge graph with gossip sync, retrieval = vector + BM25 hybrid search, llm = provider-agnostic dispatch with quantisation support, ingest = file-watcher pipeline feeding the retrieval index, rpc = typed MCP service layer for external clients.
## Crate/module map (moved from source doc comments)

Kern is the knowledge + reasoning backend. Runs as a daemon (`kern --daemon`) exposing its surface over MCP stdio and HTTP.

Modules:
- `base` — foundational types and daemon initialisation.
- `bench_support` (feature = "bench") — helpers for writing/running benchmarks.
- `commands` — CLI command handlers for the kern binary.
- `config` — daemon configuration loading and validation.
- `crdt` — CRDT data structures for knowledge-graph replication.
- `gnn` — graph neural network inference for relationship scoring.
- `gossip` — peer-to-peer gossip protocol for syncing state across nodes.
- `ingest` — file ingest pipeline feeding content into the retrieval index.
- `llm` — provider-agnostic LLM dispatch layer.
- `mcp` — MCP server implementation exposing kern over stdio/HTTP.
- `profile` — query profiling and performance measurement.
- `quant` — model quantisation utilities for reducing LLM memory footprint.
- `retrieval` — hybrid vector + BM25 search over the ingested content index.
- `rpc` — typed RPC service layer consumed by external MCP clients.
- `store` — per-data-dir store registry for multi-tenant kern instances.
- `tick` — periodic background task scheduler.
- `types` — shared domain types used across kern modules.
- `wire` — serialisation helpers for wire-format encoding/decoding.
- `test_support` (cfg test) — shared test-only helpers (ephemeral HTTP stub server, etc.).
