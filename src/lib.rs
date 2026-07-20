pub mod base;
pub mod commands;
pub mod config;
pub mod crdt;
pub(crate) mod docs;
pub mod gnn;
pub mod gossip;
pub mod hub;
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

#[cfg(test)]
mod test_support;
