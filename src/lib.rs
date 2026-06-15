//! Kern — the knowledge and reasoning backend.
//!
//! Runs as a daemon (`kern --daemon`) and exposes its surface over MCP
//! stdio and HTTP. Core responsibilities:
//!
//! * **memory** — CRDT-replicated knowledge graph with gossip sync
//! * **retrieval** — vector + BM25 hybrid search over ingested content
//! * **llm** — provider-agnostic LLM dispatch with quantisation support
//! * **ingest** — file watcher pipeline that feeds the retrieval index
//! * **rpc** — typed MCP service layer consumed by external MCP clients

/// Foundational types and daemon initialisation.
pub mod base;
/// Helpers for writing and running benchmarks.
pub mod bench_support;
/// CLI command handlers for the kern binary.
pub mod commands;
/// Daemon configuration loading and validation.
pub mod config;
/// CRDT data structures used for knowledge-graph replication.
pub mod crdt;
/// Graph neural network inference for relationship scoring.
pub mod gnn;
/// Peer-to-peer gossip protocol for syncing state across nodes.
pub mod gossip;
/// File ingest pipeline that feeds content into the retrieval index.
pub mod ingest;
/// Provider-agnostic LLM dispatch layer.
pub mod llm;
/// MCP server implementation exposing kern capabilities over stdio/HTTP.
pub mod mcp;
/// Knowledge graph service for storing and querying memories.
pub mod memory_service;
/// Runtime metrics collection and reporting.
pub mod metrics;
/// Query profiling and performance measurement.
pub mod profile;
/// Model quantisation utilities for reducing LLM memory footprint.
pub mod quant;
/// Hybrid vector + BM25 search over the ingested content index.
pub mod retrieval;
/// Typed RPC service layer consumed by external MCP clients.
pub mod rpc;
/// Per-data-dir store registry for multi-tenant kern instances.
pub mod store;
/// Periodic background task scheduler.
pub mod tick;
/// Shared domain types used across kern modules.
pub mod types;
/// Live read-only HTTP graph viewer (force-directed web UI).
pub mod viewer;
/// Serialisation helpers for wire-format encoding and decoding.
pub mod wire;
