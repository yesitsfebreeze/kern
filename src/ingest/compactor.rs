//! Out-of-band journal compactor.
//!
//! `DayJournal` rollover renames each closed day to `journal/segments/`. This
//! module drains those segment files into the SQLite archive (`history.db`)
//! crash-safely: a segment is deleted only after its rows are committed, and a
//! per-segment marker (`History::segment_done`/`mark_segment`) makes re-running
//! a no-op — so a crash between insert and delete cannot double-insert.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use journal::{Entry, History};

use crate::base::graph::GraphGnn;

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

/// Background task: every `interval`, drain dated segments into `history.db`,
/// then (when `export` is on) render an Obsidian "memory of the day" note for
/// each newly-complete past day. Runs forever; spawn on startup.
pub async fn run(
	cwd: PathBuf,
	interval: Duration,
	export: bool,
	vault: Option<PathBuf>,
	graph: Arc<RwLock<GraphGnn>>,
	llm: crate::types::LlmFunc,
) {
	let seg_dir = cwd.join(".kern").join("journal").join("segments");
	let history = match History::open(&cwd) {
		Ok(h) => h,
		Err(e) => {
			tracing::warn!(target: "kern.compactor", error = %e, "history open failed; compactor disabled");
			return;
		}
	};
	loop {
		// Days that have segments this pass, captured before draining deletes them.
		let days: Vec<String> = match std::fs::read_dir(&seg_dir) {
			Ok(rd) => {
				let paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
				group_by_day(&paths).into_keys().collect()
			}
			Err(_) => Vec::new(),
		};

		if let Err(e) = compact_once(&history, &seg_dir) {
			tracing::warn!(target: "kern.compactor", error = %e, "compactor pass failed");
		}

		if export {
			if let Some(vault) = vault.as_ref() {
				render_digests(&history, &graph, &*llm, vault, &days);
			}
		}

		tokio::time::sleep(interval).await;
	}
}

/// Render the day-memory note for each newly-complete past day in `days` not yet
/// rendered. The graph read lock is held only to build inputs — never across the
/// (slow) LLM call. A missing/empty LLM result defers the note to a later pass.
fn render_digests(
	history: &History,
	graph: &Arc<RwLock<GraphGnn>>,
	llm: &dyn Fn(&str) -> String,
	vault: &Path,
	days: &[String],
) {
	let today = journal::today();
	for day in days {
		if day.as_str() >= today.as_str() {
			continue; // only strictly-past days are complete
		}
		match history.digest_done(day) {
			Ok(true) => continue,
			Ok(false) => {}
			Err(e) => {
				tracing::warn!(target: "kern.compactor", error = %e, "digest_done check failed");
				continue;
			}
		}
		let inputs = {
			let g = match graph.read() {
				Ok(g) => g,
				Err(p) => p.into_inner(),
			};
			match crate::ingest::day_digest::build_day_inputs(history, &g, day) {
				Ok(i) => i,
				Err(e) => {
					tracing::warn!(target: "kern.compactor", error = %e, "day inputs failed");
					continue;
				}
			}
		};
		match crate::ingest::day_digest::render_markdown(&inputs, llm) {
			Some(md) => match crate::ingest::day_digest::write_day_note(vault, day, &md) {
				Ok(path) => {
					let _ = history.mark_digest(day);
					tracing::info!(target: "kern.compactor", day = %day, path = %path.display(), "wrote day digest");
				}
				Err(e) => tracing::warn!(target: "kern.compactor", error = %e, "day note write failed"),
			},
			None => { /* LLM unavailable; retry on a later pass */ }
		}
	}
}

/// Group segment files by their `YYYY-MM-DD` filename prefix. A day may have
/// multiple segments (a byte-cap rollover mid-day plus the day-change rollover).
pub(crate) fn group_by_day(paths: &[PathBuf]) -> BTreeMap<String, Vec<PathBuf>> {
	let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
	for p in paths {
		if let Some(day) = day_prefix(p) {
			out.entry(day).or_default().push(p.clone());
		}
	}
	out
}

/// Extract the leading `YYYY-MM-DD` from a `YYYY-MM-DD-<stamp>.jsonl` filename.
fn day_prefix(p: &Path) -> Option<String> {
	let stem = p.file_name()?.to_str()?;
	let mut it = stem.splitn(4, '-');
	let (y, m, d) = (it.next()?, it.next()?, it.next()?);
	let ok = y.len() == 4
		&& m.len() == 2
		&& d.len() == 2
		&& y
			.bytes()
			.chain(m.bytes())
			.chain(d.bytes())
			.all(|b| b.is_ascii_digit());
	ok.then(|| format!("{y}-{m}-{d}"))
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
			Kind::ForkOpen {
				fork_id: "f".into(),
				parent: None,
			},
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
	fn groups_segment_paths_by_day_prefix() {
		let paths = vec![
			PathBuf::from("segments/2026-06-11-100.jsonl"),
			PathBuf::from("segments/2026-06-11-200.jsonl"), // byte-cap + day-change, same day
			PathBuf::from("segments/2026-06-12-050.jsonl"),
		];
		let by_day = group_by_day(&paths);
		assert_eq!(by_day.get("2026-06-11").map(|v| v.len()), Some(2));
		assert_eq!(by_day.get("2026-06-12").map(|v| v.len()), Some(1));
	}

	#[test]
	fn compact_segment_is_idempotent() {
		let dir = tempfile::tempdir().unwrap();
		let seg = one_segment(dir.path());
		let hist = History::open(dir.path()).unwrap();

		let n1 = compact_segment(&hist, &seg).unwrap();
		let n2 = compact_segment(&hist, &seg).unwrap();
		assert!(n1 >= 1, "first compaction inserts the fork row");
		assert_eq!(
			n2, 0,
			"second compaction is a no-op (segment already marked)"
		);
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
