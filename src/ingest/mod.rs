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
pub mod dedup;
pub mod distill;
pub mod embed;
pub mod file_watcher;
pub mod outcome;
pub mod place;
pub mod session_mirror;
pub mod split;
pub mod synthesis;
pub mod worker;

pub use config::Config;
pub use outcome::{FailureReport, Outcome, OutcomeStatus};
pub use crate::types::LlmFunc;
pub use worker::Worker;
// Crate-internal: `Job` is the Worker's mpsc message (pub(crate)); re-exported
// here so in-crate producers use `ingest::Job` consistently with `ingest::Worker`
// rather than reaching into `ingest::worker::Job`.
pub(crate) use worker::Job;
