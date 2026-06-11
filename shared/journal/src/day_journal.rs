use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entry::{now_ms, Entry, Sink};

pub trait HistorySink: Send + Sync {
	fn bulk_insert(
		&self,
		entries: &[Entry],
	) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// `HistorySink` that drops everything. Used by binaries that don't want
/// the SQLite warm-store layer.
pub struct NullHistorySink;

impl HistorySink for NullHistorySink {
	fn bulk_insert(
		&self,
		_entries: &[Entry],
	) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
		Ok(())
}
}

const HEADER_VERSION: u32 = 2;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Header {
	v: u32,
	project: String,
	created_ms: u64,
	created_day: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HeaderLine {
	header: Header,
}

struct Inner {
	file: File,
	current_day: String,
	bytes_written: u64,
}

/// Default soft cap on today.jsonl size before forcing a mid-day rollover.
/// 50 MB. Override per-process via `DayJournal::set_max_bytes`.
const DEFAULT_MAX_TODAY_BYTES: u64 = 50 * 1024 * 1024;

pub struct DayJournal {
	path: PathBuf,
	project_abs: String,
	history: Arc<dyn HistorySink>,
	inner: Mutex<Inner>,
	max_bytes: std::sync::atomic::AtomicU64,
}

impl DayJournal {
	pub fn open(project_root: &Path, history: Arc<dyn HistorySink>) -> io::Result<Self> {
		let dir = project_root.join(".kern").join("journal");
		fs::create_dir_all(&dir)?;
		let path = dir.join("today.jsonl");

		let project_abs = project_root
			.canonicalize()
			.unwrap_or_else(|_| project_root.to_path_buf())
			.to_string_lossy()
			.into_owned();

		let today = today_str();

		if path.exists() {
			if let Some(existing_day) = read_header_day(&path)? {
				if existing_day != today {
					let entries = read_entries(&path)?;
					if let Err(e) = history.bulk_insert(&entries) {
						eprintln!("day_journal: history bulk_insert failed on open rollover: {e}");
					}
					write_fresh(&path, &project_abs, &today)?;
				}
			} else {
				write_fresh(&path, &project_abs, &today)?;
			}
		} else {
			write_fresh(&path, &project_abs, &today)?;
		}

		let file = OpenOptions::new().read(true).append(true).open(&path)?;
		let bytes_written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

		Ok(Self {
			path,
			project_abs,
			history,
			inner: Mutex::new(Inner {
				file,
				current_day: today,
				bytes_written,
			}),
			max_bytes: std::sync::atomic::AtomicU64::new(DEFAULT_MAX_TODAY_BYTES),
		})
}

	/// Override the within-day size cap. `0` disables the cap.
	pub fn set_max_bytes(&self, cap: u64) {
		self.max_bytes.store(cap, std::sync::atomic::Ordering::Relaxed);
}

	pub fn path(&self) -> &Path {
		&self.path
}

	pub fn scan<F: FnMut(&Entry)>(&self, mut f: F) -> io::Result<()> {
		for_each_entry(&self.path, |e| f(&e))
}

	fn rollover_locked(&self, inner: &mut Inner, today: &str) -> io::Result<()> {
		let entries = read_entries(&self.path)?;
		if let Err(e) = self.history.bulk_insert(&entries) {
			eprintln!("day_journal: history bulk_insert failed on emit rollover: {e}");
		}
		write_fresh(&self.path, &self.project_abs, today)?;
		let file = OpenOptions::new()
			.read(true)
			.append(true)
			.open(&self.path)?;
		inner.file = file;
		inner.current_day = today.to_string();
		inner.bytes_written = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
		Ok(())
}
}

impl Sink for DayJournal {
	fn emit(&self, entry: Entry) {
		let today = today_str();
		let mut inner = match self.inner.lock() {
			Ok(g) => g,
			Err(poisoned) => poisoned.into_inner(),
		};

		let cap = self.max_bytes.load(std::sync::atomic::Ordering::Relaxed);
		let needs_rollover = inner.current_day != today
			|| (cap > 0 && inner.bytes_written >= cap);
		if needs_rollover {
			if let Err(e) = self.rollover_locked(&mut inner, &today) {
				eprintln!("day_journal: rollover failed: {e}");
				return;
			}
		}

		let line = match serde_json::to_string(&entry) {
			Ok(s) => s,
			Err(e) => {
				eprintln!("day_journal: serialise failed: {e}");
				return;
			}
		};
		let line_bytes = line.len() as u64 + 1;
		if let Err(e) = inner
			.file
			.write_all(line.as_bytes())
			.and_then(|_| inner.file.write_all(b"\n"))
			.and_then(|_| inner.file.flush())
		{
			eprintln!("day_journal: write failed: {e}");
		} else {
			inner.bytes_written = inner.bytes_written.saturating_add(line_bytes);
		}
}
}

fn today_str() -> String {
	OffsetDateTime::now_local()
		.unwrap_or_else(|_| OffsetDateTime::now_utc())
		.date()
		.to_string()
}

fn read_header_day(path: &Path) -> io::Result<Option<String>> {
	let file = File::open(path)?;
	let mut reader = BufReader::new(file);
	let mut first = String::new();
	let n = reader.read_line(&mut first)?;
	if n == 0 {
		return Ok(None);
	}
	match serde_json::from_str::<HeaderLine>(first.trim_end_matches('\n')) {
		Ok(h) => Ok(Some(h.header.created_day)),
		Err(_) => Ok(None),
	}
}

fn read_entries(path: &Path) -> io::Result<Vec<Entry>> {
	let mut out = Vec::new();
	for_each_entry(path, |e| out.push(e))?;
	Ok(out)
}

/// Iterate the parsed entries of a journal file: skip the header (line 0) and
/// blank lines, parse each remaining line as an [`Entry`], and call `f` for each.
/// An unparsable line is logged and skipped so one bad line can't abort the whole
/// read. Shared by `scan` (borrows each entry) and `read_entries` (collects them).
fn for_each_entry(path: &Path, mut f: impl FnMut(Entry)) -> io::Result<()> {
	let file = File::open(path)?;
	let reader = BufReader::new(file);
	for (i, line) in reader.lines().enumerate() {
		let line = line?;
		if i == 0 || line.trim().is_empty() {
			continue;
		}
		match serde_json::from_str::<Entry>(&line) {
			Ok(entry) => f(entry),
			Err(e) => eprintln!("day_journal: skipping unparsable line {}: {e}", i + 1),
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entry::{Kind, SCHEMA_VERSION};
	use std::sync::Mutex as StdMutex;

	#[derive(Default)]
	struct CapturingHistory {
		rolled: StdMutex<Vec<Entry>>,
	}
	impl HistorySink for CapturingHistory {
		fn bulk_insert(&self, entries: &[Entry]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
			self.rolled.lock().unwrap().extend_from_slice(entries);
			Ok(())
		}
	}

	fn entry(key: &str) -> Entry {
		Entry {
			v: SCHEMA_VERSION,
			ts_ms: now_ms(),
			kind: Kind::Log,
			key: key.into(),
			payload: serde_json::json!({}),
		}
	}

	#[test]
	fn open_rolls_a_stale_day_into_history_and_rewrites_the_header() {
		let dir = tempfile::tempdir().unwrap();
		let jdir = dir.path().join(".kern").join("journal");
		std::fs::create_dir_all(&jdir).unwrap();
		let today_path = jdir.join("today.jsonl");

		// Seed a file dated in the past + two entries under that stale header.
		write_fresh(&today_path, "proj", "2000-01-01").unwrap();
		{
			let mut f = OpenOptions::new().append(true).open(&today_path).unwrap();
			for k in ["a", "b"] {
				let mut s = serde_json::to_string(&entry(k)).unwrap();
				s.push('\n');
				f.write_all(s.as_bytes()).unwrap();
			}
		}

		let hist = Arc::new(CapturingHistory::default());
		let _dj = DayJournal::open(dir.path(), hist.clone()).unwrap();

		let rolled = hist.rolled.lock().unwrap();
		assert_eq!(rolled.len(), 2, "the stale day's entries are flushed to history on open");
		assert_eq!(rolled[0].key, "a");
		drop(rolled);
		assert_ne!(
			read_header_day(&today_path).unwrap().as_deref(),
			Some("2000-01-01"),
			"today.jsonl is rewritten with a fresh header",
		);
	}

	#[test]
	fn emit_rolls_over_when_the_byte_cap_is_exceeded() {
		let dir = tempfile::tempdir().unwrap();
		let hist = Arc::new(CapturingHistory::default());
		let dj = DayJournal::open(dir.path(), hist.clone()).unwrap();
		dj.set_max_bytes(1); // tiny cap: each emit rolls over the prior content.

		dj.emit(entry("first"));
		dj.emit(entry("second"));

		// 'first' was flushed to history by the rollover that ran before 'second'.
		assert!(
			hist.rolled.lock().unwrap().iter().any(|e| e.key == "first"),
			"a cap-triggered rollover flushes earlier entries to history",
		);
	}

	#[test]
	fn cap_of_zero_disables_size_rollover() {
		// max_today_bytes == 0 means "no size cap" — emits accumulate in today.jsonl
		// and never trigger a mid-day rollover (the `cap > 0 &&` guard in emit makes
		// 0 a no-op; only a day change would roll over).
		let dir = tempfile::tempdir().unwrap();
		let hist = Arc::new(CapturingHistory::default());
		let dj = DayJournal::open(dir.path(), hist.clone()).unwrap();
		dj.set_max_bytes(0); // disabled

		for k in ["a", "b", "c", "d"] {
			dj.emit(entry(k));
		}

		assert!(
			hist.rolled.lock().unwrap().is_empty(),
			"cap=0 disables size rollover -> nothing flushed to history mid-day",
		);
		// All four entries remain in the single today.jsonl (header skipped by scan).
		let mut keys = Vec::new();
		dj.scan(|e| keys.push(e.key.clone())).unwrap();
		assert_eq!(keys, vec!["a", "b", "c", "d"].into_iter().map(String::from).collect::<Vec<_>>());
	}

	#[test]
	fn scan_visits_entries_in_order_and_skips_the_header() {
		let dir = tempfile::tempdir().unwrap();
		let dj = DayJournal::open(dir.path(), Arc::new(NullHistorySink)).unwrap();
		dj.emit(entry("x"));
		dj.emit(entry("y"));

		let mut keys = Vec::new();
		dj.scan(|e| keys.push(e.key.clone())).unwrap();
		assert_eq!(keys, vec!["x".to_string(), "y".to_string()], "header skipped, order preserved");
	}
}

fn write_fresh(path: &Path, project_abs: &str, day: &str) -> io::Result<()> {
	let header = HeaderLine {
		header: Header {
			v: HEADER_VERSION,
			project: project_abs.to_string(),
			created_ms: now_ms(),
			created_day: day.to_string(),
		},
	};
	let mut line = serde_json::to_string(&header)
		.map_err(io::Error::other)?;
	line.push('\n');

	let mut file = OpenOptions::new()
		.create(true)
		.write(true)
		.truncate(true)
		.open(path)?;
	file.write_all(line.as_bytes())?;
	file.flush()?;
	let _ = file.seek(SeekFrom::End(0));
	Ok(())
}
