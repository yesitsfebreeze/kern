use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant, SystemTime};

use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::event::{WatchEvent, WatchKind};
use crate::ignore_rules::IgnoreRules;

/// Per-path debounce window. notify on Windows fires a burst of events for a
/// single logical edit (write/metadata/close); 50 ms is wide enough to
/// coalesce those without making interactive saves feel laggy.
const DEBOUNCE: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub enum WatcherError {
	#[error("notify error: {0}")]
	Notify(#[from] notify::Error),
	#[error("watcher event channel closed")]
	Closed,
}

/// Cross-platform recursive filesystem watcher with per-path debouncing.
///
/// Drop the watcher to stop the background coalescer task; the underlying
/// notify watcher is dropped first which closes its raw-event channel and
/// causes the coalescer loop to exit cleanly.
pub struct FileWatcher {
	// Drop order matters: drop `_notify` first so the std channel closes,
	// then `_task` joins on its own.
	rx: mpsc::UnboundedReceiver<WatchEvent>,
	_notify: RecommendedWatcher,
	_task: JoinHandle<()>,
}

impl FileWatcher {
	/// Create a watcher rooted at every entry in `roots` (recursive). Events
	/// matching `ignore` are dropped before debouncing.
	pub fn new(roots: Vec<PathBuf>, ignore: IgnoreRules) -> Result<Self, WatcherError> {
		let (raw_tx, raw_rx) = std_mpsc::channel::<notify::Result<Event>>();
		let mut notify_watcher =
			notify::recommended_watcher(move |res| {
				// Best-effort: if the receiver is gone we're shutting down.
				let _ = raw_tx.send(res);
			})?;

		for root in &roots {
			notify_watcher.watch(root, RecursiveMode::Recursive)?;
		}

		let (out_tx, out_rx) = mpsc::unbounded_channel::<WatchEvent>();
		let task = spawn_coalescer(raw_rx, out_tx, ignore);

		Ok(Self { rx: out_rx, _notify: notify_watcher, _task: task })
	}

	/// Receive the next coalesced event. Returns `None` once the watcher is
	/// dropped or its background task exits.
	pub async fn next_event(&mut self) -> Option<WatchEvent> {
		self.rx.recv().await
	}

	/// Borrow the underlying receiver for callers that want to plumb it into
	/// `tokio_stream::wrappers::UnboundedReceiverStream` themselves.
	pub fn receiver(&mut self) -> &mut mpsc::UnboundedReceiver<WatchEvent> {
		&mut self.rx
	}
}

fn spawn_coalescer(
	raw_rx: std_mpsc::Receiver<notify::Result<Event>>,
	out_tx: mpsc::UnboundedSender<WatchEvent>,
	ignore: IgnoreRules,
) -> JoinHandle<()> {
	tokio::task::spawn_blocking(move || coalesce_loop(raw_rx, out_tx, ignore))
}

/// Per-path pending entry: the *latest* event seen for `path` that has not
/// yet been emitted. We replace on every new event, so the result is "last
/// write wins" within the debounce window.
struct Pending {
	event: WatchEvent,
	deadline: Instant,
}

fn coalesce_loop(
	raw_rx: std_mpsc::Receiver<notify::Result<Event>>,
	out_tx: mpsc::UnboundedSender<WatchEvent>,
	ignore: IgnoreRules,
) {
	let mut pending: HashMap<PathBuf, Pending> = HashMap::new();

	loop {
		let timeout = next_timeout(&pending);
		let recv = match timeout {
			Some(t) => raw_rx.recv_timeout(t),
			None => match raw_rx.recv() {
				Ok(v) => Ok(v),
				Err(_) => Err(std_mpsc::RecvTimeoutError::Disconnected),
			},
		};

		match recv {
			Ok(Ok(ev)) => {
				for we in translate(ev, &ignore) {
					let key = we.path.clone();
					pending.insert(
						key,
						Pending { event: we, deadline: Instant::now() + DEBOUNCE },
					);
				}
			}
			Ok(Err(err)) => {
				tracing::warn!(?err, "notify error");
			}
			Err(std_mpsc::RecvTimeoutError::Timeout) => {
				// fall through to flush
			}
			Err(std_mpsc::RecvTimeoutError::Disconnected) => {
				flush_all(&mut pending, &out_tx);
				return;
			}
		}

		flush_due(&mut pending, &out_tx);
		if out_tx.is_closed() {
			return;
		}
	}
}

fn next_timeout(pending: &HashMap<PathBuf, Pending>) -> Option<Duration> {
	let earliest = pending.values().map(|p| p.deadline).min()?;
	let now = Instant::now();
	Some(earliest.saturating_duration_since(now))
}

fn flush_due(
	pending: &mut HashMap<PathBuf, Pending>,
	out_tx: &mpsc::UnboundedSender<WatchEvent>,
) {
	let now = Instant::now();
	// Collect only the due keys (the borrow checker forbids removing from
	// `pending` while iterating it). Just the `PathBuf` keys are cloned — never
	// the `Pending` values — so this stays cheap on a large pending set.
	let due: Vec<PathBuf> = pending
		.iter()
		.filter(|(_, v)| v.deadline <= now)
		.map(|(k, _)| k.clone())
		.collect();
	for key in due {
		if let Some(p) = pending.remove(&key) {
			let _ = out_tx.send(p.event);
		}
	}
}

fn flush_all(
	pending: &mut HashMap<PathBuf, Pending>,
	out_tx: &mpsc::UnboundedSender<WatchEvent>,
) {
	for (_, p) in pending.drain() {
		let _ = out_tx.send(p.event);
	}
}

/// Convert a raw notify event into zero, one, or two [`WatchEvent`]s.
///
/// Most kinds map 1:1. `Modify(Name(Both))` (debouncer-style rename with
/// both endpoints) becomes a single `Renamed`; `From`/`To` halves are
/// emitted as `Deleted` / `Created` respectively, matching what the user
/// would observe if rename endpoints straddle the watch root.
fn translate(ev: Event, ignore: &IgnoreRules) -> Vec<WatchEvent> {
	let ts = SystemTime::now();
	let paths = ev.paths;

	let mk = |path: PathBuf, kind: WatchKind| -> Option<WatchEvent> {
		if ignore.is_ignored(&path) {
			return None;
		}
		Some(WatchEvent::new(path, kind, ts))
	};

	match ev.kind {
		EventKind::Create(CreateKind::File | CreateKind::Folder | CreateKind::Any | CreateKind::Other) => {
			paths.into_iter().filter_map(|p| mk(p, WatchKind::Created)).collect()
		}
		EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if paths.len() == 2 => {
			let mut iter = paths.into_iter();
			let from = iter.next().unwrap();
			let to = iter.next().unwrap();
			if ignore.is_ignored(&to) && ignore.is_ignored(&from) {
				return Vec::new();
			}
			// `new` pins path == to for Renamed (the path arg is overridden).
			vec![WatchEvent::new(to.clone(), WatchKind::Renamed { from, to }, ts)]
		}
		EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
			paths.into_iter().filter_map(|p| mk(p, WatchKind::Deleted)).collect()
		}
		EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
			paths.into_iter().filter_map(|p| mk(p, WatchKind::Created)).collect()
		}
		EventKind::Modify(_) => {
			paths.into_iter().filter_map(|p| mk(p, WatchKind::Modified)).collect()
		}
		EventKind::Remove(RemoveKind::File | RemoveKind::Folder | RemoveKind::Any | RemoveKind::Other) => {
			paths.into_iter().filter_map(|p| mk(p, WatchKind::Deleted)).collect()
		}
		// Access / Any / Other: not actionable for ingest.
		_ => Vec::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn ev(kind: EventKind, paths: &[&str]) -> Event {
		let mut e = Event::new(kind);
		for p in paths {
			e = e.add_path(PathBuf::from(p));
		}
		e
	}

	#[test]
	fn translate_create_file_to_created() {
		let out = translate(
			ev(EventKind::Create(CreateKind::File), &["/a.txt"]),
			&IgnoreRules::empty(),
		);
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].kind, WatchKind::Created);
		assert_eq!(out[0].path, PathBuf::from("/a.txt"));
	}

	#[test]
	fn translate_rename_both_collapses_to_single_renamed() {
		let kind = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
		let out = translate(ev(kind, &["/old.txt", "/new.txt"]), &IgnoreRules::empty());
		assert_eq!(out.len(), 1, "Both -> exactly one Renamed");
		match &out[0].kind {
			WatchKind::Renamed { from, to } => {
				assert_eq!(from, &PathBuf::from("/old.txt"));
				assert_eq!(to, &PathBuf::from("/new.txt"));
			}
			other => panic!("expected Renamed, got {other:?}"),
		}
		// `path` is the new location (what downstream ingest reads).
		assert_eq!(out[0].path, PathBuf::from("/new.txt"));
	}

	#[test]
	fn translate_rename_both_with_wrong_arity_is_not_a_rename() {
		// The `paths.len() == 2` guard: a Both event with a single path falls
		// through to the generic Modify arm (Modified), never a Renamed.
		let kind = EventKind::Modify(ModifyKind::Name(RenameMode::Both));
		let out = translate(ev(kind, &["/only.txt"]), &IgnoreRules::empty());
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].kind, WatchKind::Modified);
	}

	#[test]
	fn translate_rename_half_events_split_to_delete_and_create() {
		let from = translate(
			ev(EventKind::Modify(ModifyKind::Name(RenameMode::From)), &["/g.txt"]),
			&IgnoreRules::empty(),
		);
		assert_eq!(from[0].kind, WatchKind::Deleted, "From half -> Deleted");
		let to = translate(
			ev(EventKind::Modify(ModifyKind::Name(RenameMode::To)), &["/h.txt"]),
			&IgnoreRules::empty(),
		);
		assert_eq!(to[0].kind, WatchKind::Created, "To half -> Created");
	}

	#[test]
	fn translate_generic_modify_and_remove_map_to_expected_kinds() {
		let m = translate(
			ev(EventKind::Modify(ModifyKind::Any), &["/m.txt"]),
			&IgnoreRules::empty(),
		);
		assert_eq!(m[0].kind, WatchKind::Modified);
		let r = translate(
			ev(EventKind::Remove(RemoveKind::File), &["/r.txt"]),
			&IgnoreRules::empty(),
		);
		assert_eq!(r[0].kind, WatchKind::Deleted);
	}

	#[test]
	fn translate_non_actionable_access_events_are_dropped() {
		let out = translate(
			ev(EventKind::Access(notify::event::AccessKind::Any), &["/a.txt"]),
			&IgnoreRules::empty(),
		);
		assert!(out.is_empty(), "Access events produce no WatchEvent");
	}
}
