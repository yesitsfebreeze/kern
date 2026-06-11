use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One corpus document in a trace: an id plus the text that gets ingested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceDoc {
	pub id: String,
	pub text: String,
}

/// One query probe: the query text, the document ids that count as relevant
/// (`expected_ids`, used to score recall), and the retrieval `mode` to run it in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceQuery {
	pub id: String,
	pub query: String,
	pub expected_ids: Vec<String>,
	/// Retrieval mode (`"content"` | `"reason"` | `"hybrid"`). Optional in JSON;
	/// defaults to `"hybrid"` via [`default_mode`].
	#[serde(default = "default_mode")]
	pub mode: String,
}

fn default_mode() -> String {
	"hybrid".to_string()
}

/// A retrieval benchmark trace: a named corpus of documents plus the queries to
/// run against it after ingest. Deserialized from a JSON file by [`load`].
///
/// Expected JSON schema:
/// ```json
/// {
///   "name": "my-trace",
///   "docs": [
///     { "id": "d1", "text": "the borrow checker rejects aliased mutable refs" }
///   ],
///   "queries": [
///     { "id": "q1", "query": "borrow checker", "expected_ids": ["d1"], "mode": "hybrid" }
///   ]
/// }
/// ```
/// Every field is required except each query's `mode`, which defaults to
/// `"hybrid"` when omitted. See the round-trip test below for a minimal fixture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
	pub name: String,
	pub docs: Vec<TraceDoc>,
	pub queries: Vec<TraceQuery>,
}

pub fn load<P: AsRef<Path>>(path: P) -> Result<Trace, TraceError> {
	let data = std::fs::read_to_string(path.as_ref())
		.map_err(|e| TraceError::Io(path.as_ref().to_path_buf(), e))?;
	let trace: Trace = serde_json::from_str(&data)
		.map_err(|e| TraceError::Parse(path.as_ref().to_path_buf(), e))?;
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
		// Minimal fixture matching the documented schema; q2 omits `mode`.
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
		assert_eq!(t.queries[1].mode, "hybrid", "omitted mode defaults to hybrid");

		// Serialize → parse again yields the same structure (round-trip stable).
		let round = serde_json::to_string(&t).expect("serialize");
		let t2: Trace = serde_json::from_str(&round).expect("re-parse");
		assert_eq!(t2.queries[1].expected_ids, vec!["d1".to_string()]);
		assert_eq!(t2.queries[1].mode, "hybrid", "default survives a round-trip");
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
