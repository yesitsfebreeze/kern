//! Retrieval pipeline — query text → ranked entities (+ an optional LLM answer).
//! Sibling modules are [`answer::query`]'s stages — deliberately NOT re-exported at crate level.

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
