//! Retrieval pipeline — query text → ranked entities (+ an optional LLM answer).
//!
//! [`answer::query`] is the orchestrating entry point; the sibling modules are its
//! stages, composed roughly in this order:
//! - [`hyde`] — optional LLM HyDE expansion of the query before search.
//! - [`seed`] — seed candidate entities from the query vector (HNSW/DiskANN ANN)
//!   and the lexical (BM25) index.
//! - [`expand`] — graph expansion from those seeds (personalized PageRank /
//!   HippoRAG-style multi-hop association); [`pagerank`] is the core kernel.
//! - [`fuse`] — weighted Reciprocal Rank Fusion of the candidate lists.
//! - [`score`] / [`rerank`] — fold heat / confidence / graph signals into a final
//!   score and reorder by it.
//! - [`merge`] — combine overlapping or duplicate hits.
//! - [`diversify`] — MMR diversification so near-duplicate hits don't crowd out.
//! - [`heap`] — bounded top-k selection.
//! - [`answer`] — glue the surviving context into prose via the answer LLM.
//!
//! [`cache`] memoises whole results keyed on the raw query embedding (skipping the
//! ~30 s LLM path on a repeat / near-repeat); [`digest`] builds the SessionStart
//! recall digest. Callers go through [`answer::query`], not the individual stages
//! — which is why these are NOT flattened into a crate-level re-export; they are
//! internal pipeline steps, not a public type surface.

pub use crate::types::{EmbedFunc, LlmFunc};

pub mod answer;
pub mod cache;
pub mod digest;
pub mod diversify;
pub mod expand;
pub mod fuse;
pub mod hyde;
pub mod merge;
pub mod pagerank;
pub mod rerank;
pub mod score;
pub mod seed;
