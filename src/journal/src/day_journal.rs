use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::entry::{now_ms, Entry, Sink};

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
	inner: Mutex<Inner>,
	max_bytes: std::sync::atomic::AtomicU64,
}

impl DayJournal {
	pub fn open(project_root: &Path) -> io::Result<Self> {
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
					// Stale day on disk: archive it as a segment for the
					// out-of-band compactor, then start fresh.
					rotate_to_segment(&path, &project_abs, &today)?;
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
		self
			.max_bytes
			.store(cap, std::sync::atomic::Ordering::Relaxed);
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	pub fn scan<F: FnMut(&Entry)>(&self, mut f: F) -> io::Result<()> {
		for_each_entry(&self.path, |e| f(&e))
	}

	fn rollover_locked(&self, inner: &mut Inner, today: &str) -> io::Result<()> {
		rotate_to_segment(&self.path, &self.project_abs, today)?;
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

/// Move the closed `today.jsonl` into `segments/<closed_day>-<stamp>.jsonl` and
/// write a fresh `today.jsonl` for `today`. The closed day comes from the file's
/// own header (so a byte-cap segment and the day-change segment for the same day
/// share the `YYYY-MM-DD` prefix); `<stamp>` (`now_ms`) keeps each segment unique.
/// Archival of the segment is the out-of-band compactor's job.
fn rotate_to_segment(path: &Path, project_abs: &str, today: &str) -> io::Result<()> {
	let seg_dir = path
		.parent()
		.expect("journal path always has a parent dir")
		.join("segments");
	fs::create_dir_all(&seg_dir)?;
	let closed_day = read_header_day(path)?.unwrap_or_else(|| today.to_string());
	let seg = seg_dir.join(format!("{closed_day}-{}.jsonl", now_ms()));
	fs::rename(path, &seg)?;
	write_fresh(path, project_abs, today)?;
	Ok(())
}

impl Sink for DayJournal {
	fn emit(&self, entry: Entry) {
		let today = today_str();
		let mut inner = match self.inner.lock() {
			Ok(g) => g,
			Err(poisoned) => poisoned.into_inner(),
		};

		let cap = self.max_bytes.load(std::sync::atomic::Ordering::Relaxed);
		let needs_rollover = inner.current_day != today || (cap > 0 && inner.bytes_written >= cap);
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

/// Iterate the parsed entries of a journal file: skip the header (line 0) and
/// blank lines, parse each remaining line as an [`Entry`], and call `f` for each.
/// An unparsable line is logged and skipped so one bad line can't abort the whole
/// read. Shared by `scan` (borrows each entry) and `scan_path` (the free fn).
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

/// Scan the parsed entries of a JSONL journal file at `path`, invoking `f` for
/// each (header + blank/unparsable lines skipped). The read primitive for
/// tailing a live `today.jsonl` without opening a writable `DayJournal`.
pub fn scan_path(path: &Path, f: impl FnMut(Entry)) -> io::Result<()> {
	for_each_entry(path, f)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entry::{Kind, SCHEMA_VERSION};

	fn seg_count(root: &Path) -> usize {
		let d = root.join(".kern").join("journal").join("segments");
		std::fs::read_dir(&d).map(|it| it.count()).unwrap_or(0)
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
	fn rollover_renames_closed_day_to_a_segment() {
		let dir = tempfile::tempdir().unwrap();
		let dj = DayJournal::open(dir.path()).unwrap();
		// Accumulate under the default cap, then shrink the cap so the next emit
		// rolls the prior content into exactly one segment.
		dj.emit(entry("a"));
		dj.set_max_bytes(1);
		dj.emit(entry("b"));

		let seg_dir = dir.path().join(".kern").join("journal").join("segments");
		let segs: Vec<_> = std::fs::read_dir(&seg_dir)
			.unwrap()
			.filter_map(|e| e.ok())
			.collect();
		assert_eq!(
			segs.len(),
			1,
			"the rolled-over content became one segment file"
		);
		let name = segs[0].file_name().into_string().unwrap();
		assert!(name.ends_with(".jsonl"), "segment is a .jsonl");
		assert!(
			name.len() > "YYYY-MM-DD".len(),
			"segment name carries a day prefix + stamp"
		);
		assert!(
			dir.path().join(".kern/journal/today.jsonl").exists(),
			"fresh today.jsonl"
		);
	}

	#[test]
	fn emit_rolls_over_when_the_byte_cap_is_exceeded() {
		let dir = tempfile::tempdir().unwrap();
		let dj = DayJournal::open(dir.path()).unwrap();
		dj.emit(entry("first"));
		dj.set_max_bytes(1); // tiny cap: the next emit rolls the prior content over.
		dj.emit(entry("second"));

		// 'first' was archived into the rolled-over segment.
		let seg_dir = dir.path().join(".kern").join("journal").join("segments");
		let seg = std::fs::read_dir(&seg_dir)
			.unwrap()
			.next()
			.unwrap()
			.unwrap()
			.path();
		let mut seg_keys = Vec::new();
		scan_path(&seg, |e| seg_keys.push(e.key.clone())).unwrap();
		assert!(
			seg_keys.iter().any(|k| k == "first"),
			"segment carries the earlier entry"
		);

		// today.jsonl now holds only 'second'.
		let mut live = Vec::new();
		dj.scan(|e| live.push(e.key.clone())).unwrap();
		assert_eq!(live, vec!["second".to_string()]);
	}

	#[test]
	fn cap_of_zero_disables_size_rollover() {
		// max_today_bytes == 0 means "no size cap" — emits accumulate in today.jsonl
		// and never trigger a mid-day rollover (the `cap > 0 &&` guard in emit makes
		// 0 a no-op; only a day change would roll over).
		let dir = tempfile::tempdir().unwrap();
		let dj = DayJournal::open(dir.path()).unwrap();
		dj.set_max_bytes(0); // disabled

		for k in ["a", "b", "c", "d"] {
			dj.emit(entry(k));
		}

		assert_eq!(
			seg_count(dir.path()),
			0,
			"cap=0 disables size rollover -> no segments"
		);
		// All four entries remain in the single today.jsonl (header skipped by scan).
		let mut keys = Vec::new();
		dj.scan(|e| keys.push(e.key.clone())).unwrap();
		assert_eq!(
			keys,
			vec!["a", "b", "c", "d"]
				.into_iter()
				.map(String::from)
				.collect::<Vec<_>>()
		);
	}

	#[test]
	fn stale_day_rotates_into_a_segment_on_open() {
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

		let _dj = DayJournal::open(dir.path()).unwrap();

		// The stale day was archived as a segment named with its own date.
		let segs: Vec<_> = std::fs::read_dir(jdir.join("segments"))
			.unwrap()
			.filter_map(|e| e.ok())
			.collect();
		assert_eq!(segs.len(), 1, "stale day archived as one segment on open");
		assert!(
			segs[0]
				.file_name()
				.into_string()
				.unwrap()
				.starts_with("2000-01-01-"),
			"segment carries the stale day's date",
		);
		assert_ne!(
			read_header_day(&today_path).unwrap().as_deref(),
			Some("2000-01-01"),
			"today.jsonl is rewritten with a fresh header",
		);
	}

	#[test]
	fn scan_visits_entries_in_order_and_skips_the_header() {
		let dir = tempfile::tempdir().unwrap();
		let dj = DayJournal::open(dir.path()).unwrap();
		dj.emit(entry("x"));
		dj.emit(entry("y"));

		let mut keys = Vec::new();
		dj.scan(|e| keys.push(e.key.clone())).unwrap();
		assert_eq!(
			keys,
			vec!["x".to_string(), "y".to_string()],
			"header skipped, order preserved"
		);
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
	let mut line = serde_json::to_string(&header).map_err(io::Error::other)?;
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
