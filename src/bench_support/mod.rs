//! Benchmark + evaluation scaffolding for kern's retrieval stack.
//!
//! Bench/eval-only support compiled into the `kern` crate and driven by the bench
//! entry points (`benches/retrieval_trace_bench.rs`, `src/bin/retrieval_bench.rs`,
//! `src/bin/locomo_eval.rs`) — NOT part of the production daemon path. Modules:
//!
//! - [`locomo`] / [`locomo_run`] — the live LoCoMo conversational-memory eval:
//!   load the dataset, ingest each dialogue through the real `Worker`, answer the
//!   QA probes, and aggregate per-category quality + latency into an `EvalReport`.
//! - [`trace`] — replayable retrieval-trace JSON (corpus docs + queries + the
//!   expected ids each query should recall).
//! - [`build`] — construct a graph from a trace's documents for the harness.
//! - [`replay`] — run a trace's queries against a built graph.
//! - [`sweep`] — sweep retrieval parameters over a trace and emit CSV.
//! - [`ndcg`] — NDCG@k scoring of ranked results against the expected ids.
//! - [`embed`] — a deterministic stub embedder so trace replays are reproducible
//!   without a live embedding model.

pub mod backend;
pub mod build;
pub mod compare;
pub mod embed;
pub mod latency;
pub mod locomo;
pub mod locomo_run;
pub mod memory;
pub mod mixed;
pub mod ndcg;
pub mod replay;
pub mod stage_profile;
pub mod sweep;
pub mod trace;
