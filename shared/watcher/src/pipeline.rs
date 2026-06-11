use std::path::Path;

use tokio::sync::mpsc;

use crate::event::{WatchEvent, WatchKind};

/// Hard cap on the size of files we read into an [`IngestRecord`]. Anything
/// larger is silently skipped — the search index is for source-shaped text,
/// not blobs.
pub const MAX_INGEST_BYTES: u64 = 1024 * 1024;

/// Payload handed to a downstream sink. `source_uri` is always a `file://`
/// URI built from the absolute path; kern's `ingest` MCP tool expects this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRecord {
	pub source_uri: String,
	pub content: String,
	pub language_hint: Option<String>,
}

/// Downstream consumer. Implemented by the kern wiring (slice F) — this
/// crate must NOT depend on kern.
#[async_trait::async_trait]
pub trait IngestSink: Send + Sync + 'static {
	async fn ingest(&self, record: IngestRecord);
}

/// Drives a stream of [`WatchEvent`]s into an [`IngestSink`].
///
/// * `Created` / `Modified` → read file (≤ [`MAX_INGEST_BYTES`]) → ingest.
/// * `Deleted` → no read; ignored at this layer (kern handles deletion via
///   a separate call path; slice E only does *content* ingest).
/// * `Renamed { from, to }` → treated as `Created(to)`.
/// * Files larger than `MAX_INGEST_BYTES` or with non-UTF-8 content are
///   skipped silently (logged at `debug`).
pub struct IngestPipeline<S: IngestSink> {
	sink: S,
}

impl<S: IngestSink> IngestPipeline<S> {
	pub fn new(sink: S) -> Self {
		Self { sink }
	}

	/// Consume events from `rx` until the channel closes.
	pub async fn run(self, mut rx: mpsc::UnboundedReceiver<WatchEvent>) {
		while let Some(ev) = rx.recv().await {
			if let Some(rec) = build_record(&ev).await {
				self.sink.ingest(rec).await;
			}
		}
	}

	/// Process a single event. Exposed for tests and synchronous callers.
	pub async fn handle(&self, ev: WatchEvent) {
		if let Some(rec) = build_record(&ev).await {
			self.sink.ingest(rec).await;
		}
	}
}

async fn build_record(ev: &WatchEvent) -> Option<IngestRecord> {
	let path: &Path = match &ev.kind {
		WatchKind::Created | WatchKind::Modified => &ev.path,
		WatchKind::Renamed { to, .. } => to,
		WatchKind::Deleted => return None,
	};

	let meta = tokio::fs::metadata(path).await.ok()?;
	if !meta.is_file() {
		return None;
	}
	if meta.len() > MAX_INGEST_BYTES {
		tracing::debug!(?path, size = meta.len(), "skipping oversize file");
		return None;
	}
	let bytes = tokio::fs::read(path).await.ok()?;
	let content = match String::from_utf8(bytes) {
		Ok(s) => s,
		Err(_) => {
			tracing::debug!(?path, "skipping non-utf8 file");
			return None;
		}
	};

	Some(IngestRecord {
		source_uri: file_uri(path),
		content,
		language_hint: language_hint(path),
	})
}

fn file_uri(path: &Path) -> String {
	let abs = match path.canonicalize() {
		Ok(p) => p,
		Err(_) => path.to_path_buf(),
	};
	let s = abs.to_string_lossy().replace('\\', "/");
	// Windows canonical paths come back as `\\?\C:\foo`; normalise.
	let trimmed = s.strip_prefix("//?/").unwrap_or(&s);
	if trimmed.starts_with('/') {
		format!("file://{trimmed}")
	} else {
		format!("file:///{trimmed}")
	}
}

fn language_hint(path: &Path) -> Option<String> {
	let ext = path.extension()?.to_str()?.to_ascii_lowercase();
	let hint = match ext.as_str() {
		"rs" => "rust",
		"ts" | "tsx" => "typescript",
		"js" | "jsx" | "mjs" | "cjs" => "javascript",
		"py" => "python",
		"go" => "go",
		"md" => "markdown",
		"toml" => "toml",
		"json" => "json",
		"yaml" | "yml" => "yaml",
		_ => return Some(ext),
	};
	Some(hint.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;
	use std::time::SystemTime;

	// All file_uri cases below use paths that don't exist on disk, so
	// `canonicalize` fails and the deterministic string-normalisation fallback
	// runs — making the expected output stable across machines.

	#[test]
	fn file_uri_unix_absolute_path_gets_three_slashes() {
		assert_eq!(
			file_uri(Path::new("/nonexistent_kern_test/dir/file.rs")),
			"file:///nonexistent_kern_test/dir/file.rs"
		);
	}

	#[test]
	fn file_uri_strips_windows_unc_prefix() {
		// `\\?\C:\..` is what Windows canonicalize returns; the `//?/` prefix is
		// stripped and the drive path becomes `file:///C:/..`. (Backslashes are
		// literal chars on Unix, so the string ops are identical cross-platform.)
		assert_eq!(
			file_uri(Path::new(r"\\?\C:\foo\bar.rs")),
			"file:///C:/foo/bar.rs"
		);
	}

	#[cfg(unix)]
	fn non_utf8_path() -> PathBuf {
		use std::os::unix::ffi::OsStrExt;
		// 0x80 is an invalid UTF-8 lead byte.
		std::ffi::OsStr::from_bytes(&[0x66, 0x80, 0x66]).into()
	}

	#[cfg(windows)]
	fn non_utf8_path() -> PathBuf {
		use std::os::windows::ffi::OsStringExt;
		// 0xD800 is an unpaired surrogate -> not valid UTF-16/UTF-8.
		std::ffi::OsString::from_wide(&[0x66, 0xD800, 0x66]).into()
	}

	#[tokio::test]
	async fn renamed_with_non_utf8_from_reads_the_to_path() {
		// build_record uses the `to` endpoint of a rename; a non-UTF-8 `from`
		// path must not affect it (the from is never read or stringified).
		let dir = tempfile::tempdir().unwrap();
		let to = dir.path().join("renamed.rs");
		tokio::fs::write(&to, "fn main() {}").await.unwrap();

		let ev = WatchEvent {
			path: to.clone(),
			kind: WatchKind::Renamed { from: non_utf8_path(), to: to.clone() },
			ts: SystemTime::now(),
		};
		let rec = build_record(&ev).await.expect("record built from the `to` path");
		assert_eq!(rec.content, "fn main() {}");
		assert_eq!(rec.language_hint.as_deref(), Some("rust"));
		assert!(rec.source_uri.starts_with("file://"));
	}

	#[tokio::test]
	async fn deleted_events_build_no_record() {
		let ev = WatchEvent {
			path: PathBuf::from("/whatever.rs"),
			kind: WatchKind::Deleted,
			ts: SystemTime::now(),
		};
		assert!(build_record(&ev).await.is_none());
	}
}
