use std::path::Path;

use crate::base::types::{EntityKind, Source};
use crate::base::util;
use crate::ingest::outcome::OutcomeStatus;
use crate::ingest::Worker;

use serde::{Deserialize, Serialize};

// Serialized as serde_json (name-based) — the bincode positional law does not apply here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectJob {
	pub text: String,
	pub source: Source,
	pub kind: EntityKind,
	pub descriptor: String,
	pub confidence: f64,
}

pub fn intake_direct(direct_dir: &Path, job: &DirectJob) -> std::io::Result<String> {
	std::fs::create_dir_all(direct_dir)?;
	let doc_id = util::content_hash(&job.text);
	let dst = direct_dir.join(format!("{doc_id}.json"));
	let tmp = direct_dir.join(format!("{doc_id}.{}.tmp", std::process::id()));
	let payload = serde_json::to_vec(job).map_err(std::io::Error::other)?;
	std::fs::write(&tmp, payload)?;
	if let Err(e) = std::fs::rename(&tmp, &dst) {
		let _ = std::fs::remove_file(&tmp);
		// dst already existing (concurrent identical intake) is success.
		if !dst.exists() {
			return Err(e);
		}
	}
	Ok(doc_id)
}

pub async fn drain_direct_once(
	direct_dir: &Path,
	worker: &Worker,
	cfg: &crate::ingest::Config,
) -> usize {
	let entries = match std::fs::read_dir(direct_dir) {
		Ok(e) => e,
		Err(_) => return 0,
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
				super::intake::archive(&path, &done);
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
		super::intake::archive(&path, &done);
		archived += 1;
	}
	archived
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use parking_lot::RwLock;
	use std::sync::Arc;
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
	fn intake_direct_writes_idempotent_json_named_by_content_hash() {
		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");

		let id1 = intake_direct(&direct, &job("a durable fact")).expect("accepted");
		let id2 = intake_direct(&direct, &job("a durable fact")).expect("re-submit ok");
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

		let raw = std::fs::read_to_string(files[0].path()).unwrap();
		let back: DirectJob = serde_json::from_str(&raw).expect("valid json payload");
		assert_eq!(back.text, "a durable fact");
		assert_eq!(back.confidence, 0.7);
	}

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

		let embedder = crate::llm::Client::new_embed_only(&format!("http://{addr}"), "m", "");
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), embedder, None, None, None);

		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");
		let doc_id = intake_direct(&direct, &job("the spawn gate shipped today")).expect("accepted");

		let cfg = crate::ingest::Config {
			dedup_threshold: 0.95,
			..Default::default()
		};
		let archived = drain_direct_once(&direct, &worker, &cfg).await;

		assert_eq!(archived, 1, "the job committed -> archived");
		assert!(
			direct.join("done").join(format!("{doc_id}.json")).exists(),
			"intake file moved into direct/done/"
		);
		let g = graph.read();
		let total: usize = g.all().iter().map(|k| k.entities.len()).sum();
		assert!(
			total > 0,
			"the payload flowed through the worker into the graph"
		);

		server.abort();
	}

	#[tokio::test]
	async fn drain_direct_once_leaves_failed_job_for_retry() {
		let embedder = crate::llm::Client::new_embed_only("http://127.0.0.1:1", "m", "");
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph, embedder, None, None, None);

		let dir = tempdir().unwrap();
		let direct = dir.path().join("direct");
		let doc_id = intake_direct(&direct, &job("must survive the outage")).expect("accepted");

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
			"file left in the direct intake for retry"
		);
	}
}
