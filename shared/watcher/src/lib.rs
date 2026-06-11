//! Filesystem watcher + ingest pipeline.
//!
//! Filesystem watcher. Wraps `notify` with:
//!
//! * cross-platform recommended-watcher mode,
//! * 50 ms per-path debounce (drops intermediates),
//! * `.gitignore` + `.kernignore` honouring via the `ignore` crate
//!   (reused from `shared/search`),
//! * an [`IngestPipeline`] that reads file contents (≤1 MB) and forwards
//!   `IngestRecord`s to a downstream [`IngestSink`] (kern wires its MCP
//!   `ingest` call here).
//!
//! ## Platform quirks
//!
//! * **Windows** uses `ReadDirectoryChangesW`, which fires multiple events
//!   per logical edit (open-for-write, write, close, metadata). The 50 ms
//!   per-path debounce coalesces these into a single emitted event. Editors
//!   that swap-rename on save (vim, VS Code) appear as `Renamed { from, to }`
//!   when both endpoints are inside a watched root, otherwise as separate
//!   `Deleted` + `Created` events — this matches notify's documented
//!   behaviour and is preserved here intentionally.
//! * **macOS** FSEvents may coalesce events server-side; debounce is still
//!   applied for symmetry.
//! * **Linux** inotify fires one event per syscall; debounce mostly drops
//!   editor-induced bursts (write + chmod + close-write).
//!
//! ## Usage
//!
//! Implement an [`IngestSink`], then pump a [`FileWatcher`]'s coalesced events
//! through an [`IngestPipeline`] into it (this is exactly how kern wires it):
//!
//! ```ignore
//! use watcher::{FileWatcher, IgnoreRules, IngestPipeline, IngestRecord, IngestSink};
//!
//! struct MySink;
//! impl IngestSink for MySink {
//!     async fn ingest(&self, record: IngestRecord) {
//!         // record.source_uri + record.content -> your ingest path
//!         let _ = (record.source_uri, record.content);
//!     }
//! }
//!
//! // ...in an async context:
//! let mut watcher = FileWatcher::new(vec!["./src".into()], IgnoreRules::empty())?;
//! let pipeline = IngestPipeline::new(MySink);
//! while let Some(ev) = watcher.next_event().await {
//!     pipeline.handle(ev).await; // reads file (<= MAX_INGEST_BYTES) -> sink.ingest
//! }
//! # Ok::<(), watcher::WatcherError>(())
//! ```

mod event;
mod ignore_rules;
mod pipeline;
mod watcher;

pub use event::{WatchEvent, WatchKind};
pub use ignore_rules::IgnoreRules;
pub use pipeline::{IngestPipeline, IngestRecord, IngestSink, MAX_INGEST_BYTES};
pub use watcher::{FileWatcher, WatcherError};
