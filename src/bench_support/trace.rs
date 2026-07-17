use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceDoc {
	pub id: String,
	pub text: String,
	#[serde(default)]
	pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceQuery {
	pub id: String,
	pub query: String,
	pub expected_ids: Vec<String>,
	#[serde(default = "default_mode")]
	pub mode: String,
	#[serde(default)]
	pub filter_kind: Option<String>,
}

fn default_mode() -> String {
	"hybrid".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
	pub name: String,
	pub docs: Vec<TraceDoc>,
	pub queries: Vec<TraceQuery>,
}

pub fn load<P: AsRef<Path>>(path: P) -> Result<Trace, TraceError> {
	let data = std::fs::read_to_string(path.as_ref())
		.map_err(|e| TraceError::Io(path.as_ref().to_path_buf(), e))?;
	let trace: Trace =
		serde_json::from_str(&data).map_err(|e| TraceError::Parse(path.as_ref().to_path_buf(), e))?;
	Ok(trace)
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
	#[error("failed to read trace {}: {}", .0.display(), .1)]
	Io(PathBuf, #[source] std::io::Error),
	#[error("failed to parse trace {}: {}", .0.display(), .1)]
	Parse(PathBuf, #[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn trace_json_round_trips_and_mode_defaults_to_hybrid() {
		let json = r#"{
			"name": "t1",
			"docs": [{ "id": "d1", "text": "the borrow checker" }],
			"queries": [
				{ "id": "q1", "query": "borrow", "expected_ids": ["d1"], "mode": "content" },
				{ "id": "q2", "query": "aliasing", "expected_ids": ["d1"] }
			]
		}"#;
		let t: Trace = serde_json::from_str(json).expect("parse fixture");
		assert_eq!(t.name, "t1");
		assert_eq!(t.docs.len(), 1);
		assert_eq!(t.docs[0].id, "d1");
		assert_eq!(t.queries[0].mode, "content", "explicit mode is preserved");
		assert_eq!(
			t.queries[1].mode, "hybrid",
			"omitted mode defaults to hybrid"
		);

		let round = serde_json::to_string(&t).expect("serialize");
		let t2: Trace = serde_json::from_str(&round).expect("re-parse");
		assert_eq!(t2.queries[1].expected_ids, vec!["d1".to_string()]);
		assert_eq!(
			t2.queries[1].mode, "hybrid",
			"default survives a round-trip"
		);
	}

	#[test]
	fn load_reads_a_trace_from_disk() {
		let dir = tempfile::tempdir().unwrap();
		let p = dir.path().join("trace.json");
		std::fs::write(&p, r#"{ "name": "x", "docs": [], "queries": [] }"#).unwrap();
		let t = load(&p).expect("load ok");
		assert_eq!(t.name, "x");
		assert!(t.docs.is_empty() && t.queries.is_empty());
	}

	#[test]
	fn load_missing_file_is_an_io_error() {
		let dir = tempfile::tempdir().unwrap();
		let missing = dir.path().join("nope.json");
		assert!(matches!(load(&missing).unwrap_err(), TraceError::Io(..)));
	}

	#[test]
	fn load_malformed_json_is_a_parse_error() {
		let dir = tempfile::tempdir().unwrap();
		let p = dir.path().join("bad.json");
		std::fs::write(&p, "{ not valid json").unwrap();
		assert!(matches!(load(&p).unwrap_err(), TraceError::Parse(..)));
	}
}
