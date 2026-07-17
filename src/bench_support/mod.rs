//! Benchmark + evaluation scaffolding for kern's retrieval stack — bench/eval
//! only, NOT part of the production daemon path.

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
