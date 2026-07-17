pub mod base;
#[cfg(feature = "bench")]
pub mod bench_support;
pub mod commands;
pub mod config;
pub mod crdt;
pub mod gnn;
pub mod gossip;
pub mod ingest;
pub mod llm;
pub mod mcp;
pub mod profile;
pub mod quant;
pub mod retrieval;
pub mod rpc;
pub mod store;
pub mod tick;
pub mod types;
pub mod wire;

#[cfg(test)]
mod test_support;
