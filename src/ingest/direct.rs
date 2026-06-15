//! Durable direct-ingest lane.
//!
//! The MCP `ingest` tool's fire-and-forget path used to hand a job to the
//! in-RAM worker channel and ack `"queued"` — any daemon exit (watchdog
//! `exit(101)`, crash, operator stop) silently vaporized every queued job
//! after the ack. Observed live: 5 acked ingests lost to one restart.
//!
//! This lane reuses the capture spool's proven durability shape instead:
//! the payload is serialized to `<spool>/direct/<doc_id>.json` (atomic
//! tmp+rename, content-hash named so re-spooling the same text is
//! idempotent) BEFORE the ack — which becomes `"spooled"` — and the drain
//! cycle replays it through the canonical `Worker` with its ORIGINAL
//! source/kind/confidence, archiving the file only on a non-`Failed`
//! outcome. Unlike the `.txt` capture lane there is NO distill step: an
//! explicit ingest payload is ingested as-is.

use std::path::Path;

use crate::base::types::{EntityKind, Source};
use crate::base::util;
use crate::ingest::outcome::OutcomeStatus;
use crate::ingest::Worker;

use serde::{Deserialize, Serialize};

/// One durable direct-ingest payload: the post-clamp job parameters exactly
/// as the worker should see them. Serialized as serde_json (name-based — the
/// bincode positional law does not apply to this file format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectJob {
	pub text: String,
	pub source: Source,
	pub kind: EntityKind,
	pub descriptor: String,
	pub confidence: f64,
}

/// Persist `job` into `<direct_dir>/<doc_id>.json` atomically (tmp + rename),
/// creating the directory if needed. Returns the doc_id (content hash of the
/// text — the same id the worker will commit under on a fresh placement).
/// Content-hash naming makes re-spooling identical text idempotent. The tmp
/// file is pid-tagged so two concurrent spoolers can't clobber each other's
/// half-written payload (same shape as the capture hook's offsets write).
pub fn spool_direct(direct_dir: &Path, job: &DirectJob) -> std::io::Result<String> {
	std::fs::create_dir_all(direct_dir)?;
	let doc_id = util::content_hash(&job.text);
	let dst = direct_dir.join(format!("{doc_id}.json"));
	let tmp = direct_dir.join(format!("{doc_id}.{}.tmp", std::process::id()));
	let payload = serde_json::to_vec(job).map_err(std::io::Error::other)?;
	std::fs::write(&tmp, payload)?;
	if let Err(e) = std::fs::rename(&tmp, &dst) {
		let _ = std::fs::remove_file(&tmp);
		// The destination already existing (concurrent identical spool) is
		// success — the payload IS on disk under the right name.
		if !dst.exists() {
			return Err(e);
		}
	}
	Ok(doc_id)
}

/// Drain every `*.json` direct job under `direct_dir`: replay through
/// `worker.run` with the original parameters, archive into
/// `<direct_dir>/done/` on any non-`Failed` outcome (a `Deduped` merge is
/// success), leave the file for retry on `Failed`. Returns the number of
/// files archived. An unparseable payload is archived rather than retried —
/// it can never succeed, and retrying it forever would wedge the lane.
pub async fn drain_direct_once(
	direct_dir: &Path,
	worker: &Worker,
	cfg: &crate::ingest::Config,
) -> usize {
	let entries = match std::fs::read_dir(direct_dir) {
		Ok(e) => e,
		Err(_) => return 0, // lane unused so far — nothing spooled
	};
	let done = direct_dir.join("done");
	let mut archived = 0;
	for ent in entries.flatten() {
		let path = ent.path();
		if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
			continue;
		}
		let job: DirectJob = match std::fs::read_to_string(&path)
			.map_err(|e| e.to_string())
			.and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
		{
			Ok(j) => j,
			Err(e) => {
				tracing::warn!(
					target: "kern.ingest.direct",
					path = %path.display(),
					error = %e,
					"unreadable direct payload; archiving as poison (retry cannot succeed)"
				);
				super::capture_spool::archive(&path, &done);
				archived += 1;
				continue;
			}
		};
		let outcome = worker
			.run(
				job.text,
				job.source,
				job.kind,
				job.descriptor,
				job.confidence,
				cfg.clone(),
			)
			.await;
		if matches!(outcome.status, OutcomeStatus::Failed) {
			tracing::warn!(
				target: "kern.ingest.direct",
				path = %path.display(),
				status = outcome.status.as_str(),
				"direct ingest failed; leaving payload for retry"
			);
			continue;
		}
		super::capture_spool::archive(&path, &done);
		archived += 1;
	}
	archived
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use std::sync::{Arc, RwLock};
	use std::time::Duration;
	use tempfile::tempdir;

	fn job(text: &str) -> DirectJob {
		DirectJob {
			text: text.to_string(),
			source: Source::Inline {
				hash: "obj-1".into(),
				section: String::new(),
			},
			kind: EntityKind::Claim,
			descriptor: "audit-finding".into(),
			confidence: 0.7,
		}
	}

	#[test]
	fn spool_direct_writes_idempotent_json_named_by_content_hash() {
		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");

		let id1 = spool_direct(&direct, &job("a durable fact")).expect("spooled");
		let id2 = spool_direct(&direct, &job("a durable fact")).expect("re-spool ok");
		assert_eq!(
			id1,
			util::content_hash("a durable fact"),
			"doc id is the content hash"
		);
		assert_eq!(id1, id2, "same text -> same file, idempotent");

		let files: Vec<_> = std::fs::read_dir(&direct)
			.unwrap()
			.flatten()
			.filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
			.collect();
		assert_eq!(files.len(), 1, "one file per unique payload");

		// No tmp residue and the payload round-trips.
		let raw = std::fs::read_to_string(files[0].path()).unwrap();
		let back: DirectJob = serde_json::from_str(&raw).expect("valid json payload");
		assert_eq!(back.text, "a durable fact");
		assert_eq!(back.confidence, 0.7);
	}

	/// A direct job is replayed through a REAL worker (embeddings from a local
	/// /api/embed stub), lands in the graph with its original parameters, and
	/// the spool file is archived. No distill step is involved — the payload
	/// must arrive verbatim, not as extracted claims.
	#[tokio::test]
	async fn drain_direct_once_ingests_and_archives_end_to_end() {
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
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), embedder, None, None);

		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");
		let doc_id = spool_direct(&direct, &job("the spawn gate shipped today")).expect("spooled");

		let cfg = crate::ingest::Config {
			dedup_threshold: 0.95,
			..Default::default()
		};
		let archived = drain_direct_once(&direct, &worker, &cfg).await;

		assert_eq!(archived, 1, "the job committed -> archived");
		assert!(
			direct.join("done").join(format!("{doc_id}.json")).exists(),
			"spool file moved into direct/done/"
		);
		let g = crate::base::locks::read_recovered(&graph);
		let total: usize = g.all().iter().map(|k| k.entities.len()).sum();
		assert!(
			total > 0,
			"the payload flowed through the worker into the graph"
		);

		server.abort();
	}

	/// Embed endpoint down -> the job FAILS -> the spool file must remain for
	/// the next drain cycle. This is the whole point of the lane: a transient
	/// outage (or a daemon death before the drain) never loses an acked ingest.
	#[tokio::test]
	async fn drain_direct_once_leaves_failed_job_for_retry() {
		let embedder = crate::llm::Client::new_embed_only("http://127.0.0.1:1", "m");
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph, embedder, None, None);

		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");
		let doc_id = spool_direct(&direct, &job("must survive the outage")).expect("spooled");

		let cfg = crate::ingest::Config {
			dedup_threshold: 0.95,
			..Default::default()
		};
		let archived = tokio::time::timeout(
			Duration::from_secs(30),
			drain_direct_once(&direct, &worker, &cfg),
		)
		.await
		.expect("drain must not hang");

		assert_eq!(archived, 0, "failed job is not archived");
		assert!(
			direct.join(format!("{doc_id}.json")).exists(),
			"file left in the direct spool for retry"
		);
	}
}
