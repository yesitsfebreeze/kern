//! Filesystem watcher (notify + 50 ms per-path debounce + ignore rules) feeding
//! an [`IngestPipeline`]; files over [`MAX_INGEST_BYTES`] are silently skipped.

mod event;
mod ignore_rules;
mod pipeline;
mod watcher;

pub use event::{WatchEvent, WatchKind};
pub use ignore_rules::IgnoreRules;
pub use pipeline::{IngestPipeline, IngestRecord, IngestSink, MAX_INGEST_BYTES};
pub use watcher::{FileWatcher, WatcherError};
