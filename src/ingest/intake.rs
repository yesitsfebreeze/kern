//! Claude-Code capture intake.
//!
//! The CC `Stop` hook drops plain-text conversation deltas into the intake
//! directory. This task drains them: each delta is distilled into durable
//! `Claim`s (LLM), each claim is ingested through the canonical `Worker`,
//! and the consumed file is archived to `<intake>/done/` — but ONLY once every
//! claim has ingested successfully. If ingest fails (e.g. the embed endpoint
//! is down) the delta is left in the intake and retried on the next drain, so
//! a transient LLM outage never loses captured knowledge.
//!
//! The daemon is the single graph owner, so ingest happens in-process with
//! no CLI race.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::base::types::{EntityKind, Source};
use crate::ingest::distill::{distill, Claim};
use crate::ingest::outcome::OutcomeStatus;
use crate::ingest::Worker;
use crate::types::LlmFunc;

/// Read + distill one delta file into `(stem, claims)`. The stem is the file
/// name without extension (used as the session id for provenance). Returns
/// `None` on read failure OR when distillation got no output from the LLM (a
/// transient outage) — in both cases the file is left in place for retry.
pub fn extract_claims(path: &Path, llm: &dyn Fn(&str) -> String) -> Option<(String, Vec<Claim>)> {
	let text = match std::fs::read_to_string(path) {
		Ok(t) => t,
		Err(e) => {
			tracing::warn!(target: "kern.ingest.intake", path = %path.display(), error = %e, "failed to read delta; leaving in intake");
			return None;
		}
	};
	let stem = path
		.file_stem()
		.and_then(|s| s.to_str())
		.unwrap_or("claude")
		.to_string();
	let claims = match distill(&text, llm) {
		Some(c) => c,
		None => {
			tracing::warn!(target: "kern.ingest.intake", path = %path.display(), "distill got no LLM output; leaving delta in intake for retry");
			return None;
		}
	};
	Some((stem, claims))
}

/// Move a fully-ingested delta into `<done>/`. Best effort: on rename failure
/// (e.g. cross-device) the source is removed so it is not re-processed.
pub fn archive(path: &Path, done_dir: &Path) {
	let _ = std::fs::create_dir_all(done_dir);
	if let Some(name) = path.file_name() {
		if std::fs::rename(path, done_dir.join(name)).is_err() {
			let _ = std::fs::remove_file(path);
		}
	}
}

/// Archive `path` iff every claim ingested successfully (`results` all true),
/// or there were no claims at all. Returns whether the file was archived.
/// A delta with any failed claim is left in the intake for a later retry.
pub fn finalize(path: &Path, done_dir: &Path, results: &[bool]) -> bool {
	if results.iter().all(|&ok| ok) {
		archive(path, done_dir);
		true
	} else {
		false
	}
}

/// Delete archived deltas under `done_dir` whose mtime is older than `max_age`.
/// The graph is the durable copy after ingest, so the archive is only a
/// transient audit trail; sweeping it each drain cycle bounds disk/inode
/// growth on a daemon that runs indefinitely. `now` is injected for testing.
/// Returns the number of files removed.
pub fn prune_done(done_dir: &Path, max_age: Duration, now: SystemTime) -> usize {
	let entries = match std::fs::read_dir(done_dir) {
		Ok(e) => e,
		Err(_) => return 0, // archive dir may not exist yet
	};
	let mut removed = 0;
	for ent in entries.flatten() {
		let path = ent.path();
		if !path.is_file() {
			continue;
		}
		let modified = match ent.metadata().and_then(|m| m.modified()) {
			Ok(m) => m,
			Err(_) => continue,
		};
		let too_old = now
			.duration_since(modified)
			.map(|age| age > max_age)
			.unwrap_or(false);
		if too_old && std::fs::remove_file(&path).is_ok() {
			removed += 1;
		}
	}
	removed
}

/// Distill + ingest one `*.txt` delta, archiving it into `done/` iff every claim
/// committed. Returns whether the file was archived. A non-`.txt` path, a read
/// failure, an LLM outage (no distill output), or any failed claim leaves the
/// delta in the intake for the next cycle to retry — so transient outages never
/// drop captured knowledge.
async fn drain_entry(
	path: &Path,
	done: &Path,
	worker: &Worker,
	llm: &LlmFunc,
	cfg: &crate::ingest::Config,
) -> bool {
	if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("txt") {
		return false;
	}
	let (stem, claims) = match extract_claims(path, llm.as_ref()) {
		Some(v) => v,
		None => return false,
	};
	let mut results = Vec::with_capacity(claims.len());
	for c in claims {
		let src = Source::Session {
			session_id: format!("claude:{stem}"),
			section: String::new(),
			title: format!("claude://{}", c.descriptor),
		};
		// Carry the distilled world-time hint onto this claim's ingest config.
		let mut claim_cfg = cfg.clone();
		claim_cfg.valid_from = c.valid_from;
		let outcome = worker
			.run(
				c.text,
				src,
				EntityKind::Claim,
				c.descriptor,
				0.6,
				claim_cfg,
			)
			.await;
		let ok = !matches!(outcome.status, OutcomeStatus::Failed);
		if !ok {
			tracing::warn!(target: "kern.ingest.intake", stem = %stem, status = outcome.status.as_str(), "claim ingest failed; leaving delta for retry");
		}
		results.push(ok);
	}
	finalize(path, done, &results)
}

/// Process one drain cycle: [`drain_entry`] every delta in `intake_dir`, then
/// prune `done/` of entries older than `done_retention`. Returns the number of
/// deltas archived.
///
/// Split out of [`run`]'s poll loop so the full intake→distill→ingest→archive
/// path is unit-testable without spawning the never-returning loop. `now` is
/// injected for the prune step's age comparison.
async fn drain_once(
	intake_dir: &Path,
	done: &Path,
	worker: &Worker,
	llm: &LlmFunc,
	cfg: &crate::ingest::Config,
	done_retention: Duration,
	now: SystemTime,
) -> usize {
	let entries = match std::fs::read_dir(intake_dir) {
		Ok(e) => e,
		Err(e) => {
			tracing::warn!(target: "kern.ingest.intake", dir = %intake_dir.display(), error = %e, "failed to read intake dir");
			return 0;
		}
	};
	let mut archived = 0;
	for ent in entries.flatten() {
		if drain_entry(&ent.path(), done, worker, llm, cfg).await {
			archived += 1;
		}
	}
	// The durable direct-ingest lane shares this drain cycle: MCP `ingest`
	// payloads accepted under `<intake>/direct/` replay through the same worker
	// (verbatim — no distill) and archive into `direct/done/` on success.
	archived += super::direct::drain_direct_once(&intake_dir.join("direct"), worker, cfg).await;
	prune_done(done, done_retention, now);
	prune_done(&intake_dir.join("direct").join("done"), done_retention, now);
	archived
}

/// Daemon loop. Drains `intake_dir` immediately on startup, then every
/// `interval`, via [`drain_once`]: for each `*.txt` delta — distill, ingest each
/// claim through `worker` (awaiting the outcome), and archive the file only if
/// all claims committed. Each cycle also prunes archived deltas older than
/// `done_retention` so `done/` stays bounded.
pub async fn run(
	intake_dir: PathBuf,
	worker: Arc<Worker>,
	llm: LlmFunc,
	dedup_threshold: f64,
	interval: Duration,
	done_retention: Duration,
) {
	let _ = std::fs::create_dir_all(&intake_dir);
	let done = intake_dir.join("done");
	let cfg = crate::ingest::Config {
		dedup_threshold,
		..Default::default()
	};
	// Drain first, sleep after — a delta dropped before the daemon started is
	// processed on the first cycle instead of waiting a full `interval`.
	loop {
		drain_once(
			&intake_dir,
			&done,
			&worker,
			&llm,
			&cfg,
			done_retention,
			SystemTime::now(),
		)
		.await;
		tokio::time::sleep(interval).await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::{Duration, SystemTime};
	use tempfile::tempdir;

	#[test]
	fn prune_done_removes_entries_older_than_retention() {
		let dir = tempdir().unwrap();
		let done = dir.path().to_path_buf();
		let f = done.join("old.txt");
		std::fs::write(&f, "x").unwrap();
		// Treat "now" as an hour past the file's mtime, retention 60s -> pruned.
		let future = SystemTime::now() + Duration::from_secs(3600);
		let removed = prune_done(&done, Duration::from_secs(60), future);
		assert_eq!(removed, 1);
		assert!(!f.exists());
	}

	#[test]
	fn prune_done_keeps_recent_entries() {
		let dir = tempdir().unwrap();
		let done = dir.path().to_path_buf();
		let f = done.join("fresh.txt");
		std::fs::write(&f, "x").unwrap();
		let removed = prune_done(&done, Duration::from_secs(3600), SystemTime::now());
		assert_eq!(removed, 0);
		assert!(f.exists());
	}

	#[test]
	fn prune_done_missing_dir_is_noop() {
		let dir = tempdir().unwrap();
		let missing = dir.path().join("nope");
		assert_eq!(
			prune_done(&missing, Duration::from_secs(1), SystemTime::now()),
			0
		);
	}

	fn stub_two(_q: &str) -> String {
		r#"[{"text":"fact one","kind":"fact"},{"text":"a preference","kind":"preference"}]"#.to_string()
	}

	#[test]
	fn extract_reads_and_distills() {
		let dir = tempdir().unwrap();
		let delta = dir.path().join("sess-1.txt");
		std::fs::write(&delta, "user: hi\nassistant: here is a fact").unwrap();
		let (stem, claims) = extract_claims(&delta, &stub_two).expect("some");
		assert_eq!(stem, "sess-1");
		assert_eq!(claims.len(), 2);
	}

	#[test]
	fn extract_missing_file_is_none() {
		let dir = tempdir().unwrap();
		let missing = dir.path().join("nope.txt");
		assert!(extract_claims(&missing, &stub_two).is_none());
	}

	#[test]
	fn extract_returns_none_on_llm_outage() {
		// LLM outage: complete_func returns "". A non-empty delta must NOT be
		// archived as done — extract_claims returns None so run() leaves it in
		// the intake for retry. Regression guard for the data-loss bug.
		let dir = tempdir().unwrap();
		let delta = dir.path().join("sess-outage.txt");
		std::fs::write(&delta, "user: remember my API key lives in vault X").unwrap();
		let down = |_q: &str| String::new();
		assert!(extract_claims(&delta, &down).is_none());
		assert!(delta.exists(), "delta must remain for retry after outage");
	}

	#[test]
	fn extract_returns_some_on_genuine_no_claims() {
		// LLM responded "[]" (nothing worth keeping) — this is success, so the
		// delta should be archivable: extract_claims returns Some([]).
		let dir = tempdir().unwrap();
		let delta = dir.path().join("sess-empty.txt");
		std::fs::write(&delta, "user: hi\nassistant: hello").unwrap();
		let nothing = |_q: &str| "[]".to_string();
		let (stem, claims) = extract_claims(&delta, &nothing).expect("some");
		assert_eq!(stem, "sess-empty");
		assert!(claims.is_empty());
	}

	#[test]
	fn finalize_archives_when_all_ok() {
		let dir = tempdir().unwrap();
		let intake = dir.path().to_path_buf();
		let done = intake.join("done");
		let delta = intake.join("sess-1.txt");
		std::fs::write(&delta, "x").unwrap();
		assert!(finalize(&delta, &done, &[true, true]));
		assert!(!delta.exists());
		assert!(done.join("sess-1.txt").exists());
	}

	#[test]
	fn finalize_archives_when_no_claims() {
		let dir = tempdir().unwrap();
		let intake = dir.path().to_path_buf();
		let done = intake.join("done");
		let delta = intake.join("sess-2.txt");
		std::fs::write(&delta, "x").unwrap();
		assert!(finalize(&delta, &done, &[]));
		assert!(done.join("sess-2.txt").exists());
	}

	#[test]
	fn finalize_skips_archive_when_any_fail() {
		let dir = tempdir().unwrap();
		let intake = dir.path().to_path_buf();
		let done = intake.join("done");
		let delta = intake.join("sess-3.txt");
		std::fs::write(&delta, "x").unwrap();
		assert!(!finalize(&delta, &done, &[true, false]));
		assert!(delta.exists(), "delta left in intake for retry");
		assert!(!done.join("sess-3.txt").exists());
	}

	/// End-to-end drive of one drain cycle (the body of `run`'s forever loop):
	/// a seeded delta is distilled, every claim ingested through a REAL Worker,
	/// and the file archived on success — with no network (embeddings come from a
	/// local /api/embed stub) and no spawning of the never-returning loop.
	#[tokio::test]
	async fn drain_once_ingests_a_delta_and_archives_it_end_to_end() {
		use crate::base::graph::GraphGnn;
		use parking_lot::RwLock;

		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|_b: axum::Json<serde_json::Value>| async move {
				axum::Json(serde_json::json!({ "embeddings": [[0.1, 0.2, 0.3]] }))
			}),
		);
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let addr = listener.local_addr().unwrap();
		let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

		let embedder = crate::llm::Client::new_embed_only(&format!("http://{addr}"), "m");
		// Mock LLM: returns a one-claim array. distill parses it; the chunk splitter
		// receives the same string and falls back to heuristic splitting.
		let llm: LlmFunc =
			Arc::new(|_p: &str| r#"[{"text":"the API key lives in vault X","kind":"fact"}]"#.to_string());
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Arc::new(Worker::new(graph.clone(), embedder, None, None, None));

		let dir = tempdir().unwrap();
		let intake = dir.path().to_path_buf();
		let done = intake.join("done");
		let delta = intake.join("sess-42.txt");
		std::fs::write(&delta, "user: where is my key\nassistant: vault X").unwrap();

		let cfg = crate::ingest::Config {
			dedup_threshold: 0.95,
			..Default::default()
		};
		let archived = drain_once(
			&intake,
			&done,
			&worker,
			&llm,
			&cfg,
			Duration::from_secs(3600),
			SystemTime::now(),
		)
		.await;

		assert_eq!(
			archived, 1,
			"the delta's single claim committed -> archived"
		);
		assert!(!delta.exists(), "consumed delta left the intake");
		assert!(done.join("sess-42.txt").exists(), "delta moved into done/");
		let g = crate::base::locks::read_recovered(&graph);
		let entities: usize = g.all().iter().map(|k| k.entities.len()).sum();
		assert!(
			entities > 0,
			"the claim flowed through the worker into the graph"
		);

		server.abort();
	}
}
