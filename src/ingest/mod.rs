//! Ingest pipeline: turn raw text into placed, deduplicated graph entities.
//!
//! Everything is driven by [`Worker`] (an async mpsc actor — see [`worker`]),
//! which runs each document through:
//! 1. [`split`] — chunk the document into statement-sized pieces (LLM-assisted,
//!    with a heuristic fallback).
//! 2. [`embed`] — vectorize the document and each chunk via the embed endpoint.
//! 3. [`place`] — insert each piece into the owning kern, consulting [`dedup`]
//!    first so a near-duplicate vector merges into the existing entity instead of
//!    spawning a divergent one.
//! 4. [`synthesis`] — opportunistic rephrase/merge of near-but-not-duplicate
//!    neighbours. [`outcome`] reports per-document success / partial / failure.
//!
//! Ambient document sources that feed the Worker: [`capture_spool`] (the
//! Claude-Code Stop-hook spool) and [`file_watcher`]; [`session_mirror`] dedups
//! forked sessions; [`distill`] extracts durable claims from conversation text.

pub mod capture_spool;
pub mod compactor;
pub mod config;
pub mod day_digest;
pub mod dedup;
pub mod direct;
pub mod distill;
pub mod embed;
pub mod file_watcher;
pub mod outcome;
pub mod place;
pub mod session_mirror;
pub mod split;
pub mod synthesis;
pub mod worker;

pub use crate::types::LlmFunc;
pub use config::Config;
pub use outcome::{FailureReport, Outcome, OutcomeStatus};
pub use worker::Worker;
// Crate-internal: `Job` is the Worker's mpsc message (pub(crate)); re-exported
// here so in-crate producers use `ingest::Job` consistently with `ingest::Worker`
// rather than reaching into `ingest::worker::Job`.
pub(crate) use worker::Job;

/// Test-only embedder: a 256-dim one-hot unit vector derived from `seed`'s
/// content hash. Two distinct seeds almost certainly land in different slots,
/// so cosine similarity is ~0 and `commit_entity`'s dedup check (similarity >
/// threshold) is dodged. Production paths use real embeddings.
#[cfg(test)]
pub(crate) fn stub_one_hot(seed: &str) -> Vec<f64> {
	let h = crate::base::util::content_hash(seed);
	let bytes = h.as_bytes();
	let slot = if bytes.is_empty() { 0 } else { bytes[0] as usize };
	let mut v = vec![0.0_f64; 256];
	v[slot] = 1.0;
	v
}
