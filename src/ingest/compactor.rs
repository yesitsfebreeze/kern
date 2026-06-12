//! Out-of-band journal compactor.
//!
//! `DayJournal` rollover renames each closed day to `journal/segments/`. This
//! module drains those segment files into the SQLite archive (`history.db`)
//! crash-safely: a segment is deleted only after its rows are committed, and a
//! per-segment marker (`History::segment_done`/`mark_segment`) makes re-running
//! a no-op — so a crash between insert and delete cannot double-insert.

use std::path::{Path, PathBuf};
use std::time::Duration;

use journal::{Entry, History};

/// Insert a segment's entries into the archive exactly once. Returns the number
/// of rows inserted (0 if the segment was already compacted). The caller deletes
/// the file only after this returns `Ok` — a crash before the delete just
/// re-runs this, and the marker makes the insert a no-op.
pub(crate) fn compact_segment(history: &History, seg: &Path) -> anyhow::Result<usize> {
	let name = seg
		.file_name()
		.and_then(|s| s.to_str())
		.unwrap_or_default()
		.to_string();
	if history.segment_done(&name)? {
		return Ok(0);
	}
	let mut entries: Vec<Entry> = Vec::new();
	journal::scan_path(seg, |e| entries.push(e))?;
	history.bulk_insert(&entries)?;
	history.mark_segment(&name)?;
	Ok(entries.len())
}

/// Compact every `*.jsonl` segment in `seg_dir` into the archive, deleting each
/// after a successful insert. Returns the count of segments compacted. A failure
/// on one segment is logged and skipped (the file stays for the next pass).
pub(crate) fn compact_once(history: &History, seg_dir: &Path) -> anyhow::Result<usize> {
	if !seg_dir.exists() {
		return Ok(0);
	}
	let mut paths: Vec<PathBuf> = std::fs::read_dir(seg_dir)?
		.filter_map(|e| e.ok().map(|e| e.path()))
		.filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false))
		.collect();
	paths.sort();
	let mut done = 0;
	for p in &paths {
		match compact_segment(history, p) {
			Ok(_) => match std::fs::remove_file(p) {
				Ok(()) => done += 1,
				Err(e) => tracing::warn!(target: "kern.compactor", error = %e, "segment delete failed"),
			},
			Err(e) => tracing::warn!(target: "kern.compactor", error = %e, "segment compaction failed"),
		}
	}
	Ok(done)
}

/// Background task: every `interval`, drain dated segments into `history.db`.
/// Runs forever; spawn on startup. `export`/`vault` gate the Obsidian daily
/// digest, which is wired in a later task.
pub async fn run(cwd: PathBuf, interval: Duration, export: bool, vault: Option<PathBuf>) {
	let seg_dir = cwd.join(".kern").join("journal").join("segments");
	let history = match History::open(&cwd) {
		Ok(h) => h,
		Err(e) => {
			tracing::warn!(target: "kern.compactor", error = %e, "history open failed; compactor disabled");
			return;
		}
	};
	// Digest rendering (export + vault) is wired in a later task.
	let _ = (export, &vault);
	loop {
		if let Err(e) = compact_once(&history, &seg_dir) {
			tracing::warn!(target: "kern.compactor", error = %e, "compactor pass failed");
		}
		tokio::time::sleep(interval).await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use journal::{DayJournal, Entry, Kind, Sink};

	/// Emit one fork event, then force a rollover so it lands in a segment;
	/// return the segment path.
	fn one_segment(dir: &Path) -> PathBuf {
		let dj = DayJournal::open(dir).unwrap();
		dj.emit(Entry::new(
			Kind::ForkOpen { fork_id: "f".into(), parent: None },
			"mux",
			serde_json::json!({ "fork_id": "f" }),
		));
		dj.set_max_bytes(1);
		dj.emit(Entry::new(Kind::Log, "k", serde_json::Value::Null)); // rolls the fork into a segment
		std::fs::read_dir(dir.join(".kern/journal/segments"))
			.unwrap()
			.next()
			.unwrap()
			.unwrap()
			.path()
	}

	#[test]
	fn compact_segment_is_idempotent() {
		let dir = tempfile::tempdir().unwrap();
		let seg = one_segment(dir.path());
		let hist = History::open(dir.path()).unwrap();

		let n1 = compact_segment(&hist, &seg).unwrap();
		let n2 = compact_segment(&hist, &seg).unwrap();
		assert!(n1 >= 1, "first compaction inserts the fork row");
		assert_eq!(n2, 0, "second compaction is a no-op (segment already marked)");
	}

	#[test]
	fn compact_once_drains_and_deletes_segments() {
		let dir = tempfile::tempdir().unwrap();
		let _seg = one_segment(dir.path());
		let seg_dir = dir.path().join(".kern/journal/segments");
		assert_eq!(std::fs::read_dir(&seg_dir).unwrap().count(), 1);

		let hist = History::open(dir.path()).unwrap();
		let drained = compact_once(&hist, &seg_dir).unwrap();
		assert_eq!(drained, 1, "one segment compacted");
		assert_eq!(
			std::fs::read_dir(&seg_dir).unwrap().count(),
			0,
			"segment deleted after successful compaction",
		);
		// The fork row is now queryable from the archive.
		assert!(hist.len().unwrap() >= 1, "archive holds the compacted rows");
	}
}
