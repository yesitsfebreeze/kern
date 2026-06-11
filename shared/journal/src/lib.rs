//! Shared append-only JSONL event journal.
//!
//! Every binary (repl, agnt, kern, plugins) emits into the same per-day file so
//! events from each process land together. External consumers attach their own
//! sinks via the [`Sink`] trait.

pub mod day_journal;
pub mod entry;
pub mod events;
pub mod history;
pub mod state;
pub mod tracing_layer;
pub use events::{
	EntityTouchedEvent, EntityTouchedPayload, FinalEvent, FinalPayload, PlanEvent,
	PlanProposalPayload, PlanStatus, PlanStepPayload, ToolCallEvent, ToolCallPayload, TouchOp,
	TurnEndEvent, TurnEndPayload, TurnStartEvent, TurnStartPayload, system_time_from_ms,
};
pub use tracing_layer::{FieldRecorder, JournalTracingLayer};

pub use day_journal::{DayJournal, HistorySink, NullHistorySink};

/// Open the workspace-default journal: `<cwd>/.kern/journal/today.jsonl`
/// with a `NullHistorySink` (no warm SQLite store). Use this from any
/// binary that wants to emit cross-process events into the shared file.
pub fn open_default() -> std::io::Result<DayJournal> {
	let cwd = std::env::current_dir()?;
	DayJournal::open(&cwd, std::sync::Arc::new(NullHistorySink))
}

use std::sync::OnceLock;

static GLOBAL: OnceLock<DayJournal> = OnceLock::new();

/// Lazily open and return the process-global journal handle. Subsequent
/// calls reuse the same `DayJournal`. Returns `None` if open failed
/// (e.g. cwd has no `.kern`); callers should treat journaling as best-
/// effort and silently no-op on failure.
pub fn global() -> Option<&'static DayJournal> {
	if let Some(j) = GLOBAL.get() {
		return Some(j);
	}
	match open_default() {
		Ok(j) => Some(GLOBAL.get_or_init(|| j)),
		Err(_) => None,
	}
}

/// Convenience: emit a single entry via the global journal. No-op on open
/// failure.
pub fn emit(entry: Entry) {
		if let Some(j) = global() {
			j.emit(entry);
		}
}

/// `Sink` that forwards every emit to `global()`. Plug into anything that
/// already takes `Arc<dyn Sink>` to land its events in the shared journal.
pub struct GlobalSink;

impl Sink for GlobalSink {
	fn emit(&self, entry: Entry) {
		if let Some(j) = global() {
			j.emit(entry);
		}
}
}
pub use entry::{Entry, Kind, NullSink, Sink, SCHEMA_VERSION, now_ms};
pub use history::{Filter, History};
pub use state::{State, StateHandle};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn global_returns_a_consistent_handle() {
		// `global()` memoizes via a OnceLock: repeated calls must yield the SAME
		// handle (or consistently `None` when no `.kern` exists in the test cwd).
		// Comparing by pointer avoids needing a real journal dir.
		let a = global().map(|j| j as *const DayJournal);
		let b = global().map(|j| j as *const DayJournal);
		assert_eq!(a, b, "global() is idempotent across calls");
	}

	#[test]
	fn emit_and_global_sink_are_panic_safe_when_journal_absent() {
		// Both paths are documented best-effort: a failed/absent global journal
		// must silently no-op, never panic.
		emit(Entry::new(Kind::Log, "k", serde_json::Value::Null));
		GlobalSink.emit(Entry::new(Kind::Log, "k", serde_json::Value::Null));
	}
}
