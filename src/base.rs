//! Foundational layer the rest of the daemon builds on: the in-memory knowledge
//! graph, the LMDB store + cold tier, the HNSW / DiskANN vector and BM25 lexical
//! indices, CRDT merge, heat decay, and the shared types / constants / math
//! primitives.

pub mod accept;
pub mod constants;
pub mod descriptors;
pub mod diskann;
pub mod graph;
pub mod health;
pub mod heat;
pub mod hnsw;
pub mod lexical;
pub mod locks;
pub mod math;
pub mod merge;
pub mod migrate;
pub mod persist;
pub mod reason;
pub mod search;
pub mod store;
pub mod time;
pub mod types;
pub mod util;
pub mod vector_backend;
