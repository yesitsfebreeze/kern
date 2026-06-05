use crate::base::graph::GraphGnn;
use crate::base::types::*;
use crate::base::util;
use crate::ingest::config::Config;
use crate::ingest::embed::embed_chunks;
use crate::ingest::outcome::{FailureReport, Outcome, OutcomeStatus};
use crate::ingest::place::{place_chunks, place_document};
use crate::ingest::split;
use crate::llm::Client as LlmClient;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

use crate::types::LlmFunc;

pub(crate) struct Job {
	pub(crate) text: String,
	pub(crate) source: Source,
	pub(crate) kind: EntityKind,
	pub(crate) descriptor: String,
	pub(crate) confidence: f64,
	pub(crate) config: Config,
	pub(crate) result_tx: Option<oneshot::Sender<Outcome>>,
}

pub struct Worker {
	tx: mpsc::Sender<Job>,
}

impl Worker {
	pub fn new(
		graph: Arc<RwLock<GraphGnn>>,
		embedder: LlmClient,
		llm: Option<LlmFunc>,
		save_fn: Option<Arc<dyn Fn() + Send + Sync>>,
	) -> Self {
let (tx, rx) = mpsc::channel(64);
		tokio::spawn(run_loop(graph, embedder, llm, save_fn, rx));
		Self { tx }
}

	pub fn enqueue(
		&self,
		text: String,
		source: Source,
		kind: EntityKind,
		descriptor: String,
		confidence: f64,
		config: Config,
	) -> String {
let doc_id = util::content_hash(&text);
		let job = Job {
			text,
			source,
			kind,
			descriptor,
			confidence,
			config,
			result_tx: None,
		};
		let tx = self.tx.clone();
		tokio::spawn(async move {
			let _ = tx.send(job).await;
		});
		doc_id
}

	pub async fn run(
		&self,
		text: String,
		source: Source,
		kind: EntityKind,
		descriptor: String,
		confidence: f64,
		config: Config,
	) -> Outcome {
let (result_tx, result_rx) = oneshot::channel();
		let job = Job {
			text,
			source,
			kind,
			descriptor,
			confidence,
			config,
			result_tx: Some(result_tx),
		};
		if let Err(e) = self.tx.send(job).await {
			return Outcome::failed(
				"failed to enqueue",
				vec![FailureReport {
					scope: "document".into(),
					chunk_index: 0,
					class: "permanent".into(),
					error: format!("send failed: {e}"),
				}],
			);
		}
		result_rx
			.await
			.unwrap_or_else(|_| Outcome::failed("worker dropped", Vec::new()))
}
}

async fn run_loop(
	graph: Arc<RwLock<GraphGnn>>,
	embedder: LlmClient,
	llm: Option<LlmFunc>,
	save_fn: Option<Arc<dyn Fn() + Send + Sync>>,
	mut rx: mpsc::Receiver<Job>,
) {
while let Some(job) = rx.recv().await {
		let outcome = process(&graph, &embedder, &llm, &job).await;
		if let Some(sf) = &save_fn {
			sf();
		}
		if let Some(tx) = job.result_tx {
			let _ = tx.send(outcome);
		}
	}
}

async fn process(
	graph: &Arc<RwLock<GraphGnn>>,
	embedder: &LlmClient,
	llm: &Option<LlmFunc>,
	job: &Job,
) -> Outcome {
let doc_id = util::content_hash(&job.text);

	let chunks = split::split(
		&job.text,
		&job.descriptor,
		llm.as_ref().map(|f| f.as_ref() as &dyn Fn(&str) -> String),
	);

	let (doc_thought, doc_fail) =
		place_document(graph, embedder, job, &doc_id, job.config.dedup_threshold).await;
	if doc_thought.is_none() {
		let fail = doc_fail.unwrap_or(FailureReport {
			scope: "document".into(),
			chunk_index: 0,
			class: "permanent".into(),
			error: "unknown".into(),
		});
		return Outcome {
			status: OutcomeStatus::Failed,
			doc_id,
			total_chunks: chunks.len(),
			embedded_chunks: 0,
			failed_chunks: chunks.len(),
			transient_failures: if fail.class == "transient" { 1 } else { 0 },
			permanent_failures: if fail.class != "transient" { 1 } else { 0 },
			failures: vec![fail],
			message: "document embedding failed".into(),
		};
	}

	let (chunk_vecs, failures) = embed_chunks(embedder, &chunks).await;

	let placed = place_chunks(
		graph,
		llm,
		job,
		&chunks,
		&chunk_vecs,
		&doc_id,
		job.config.dedup_threshold,
	);

	let embedded_chunks = chunk_vecs.iter().filter(|v| !v.is_empty()).count();
	let failed_chunks = chunks.len() - embedded_chunks;
	let transient = failures.iter().filter(|f| f.class == "transient").count();
	let permanent = failures.iter().filter(|f| f.class != "transient").count();

	let status = if failed_chunks == 0 {
		OutcomeStatus::Committed
	} else if embedded_chunks > 0 {
		OutcomeStatus::Partial
	} else {
		OutcomeStatus::Failed
	};

	Outcome {
		status,
		doc_id,
		total_chunks: chunks.len(),
		embedded_chunks,
		failed_chunks,
		transient_failures: transient,
		permanent_failures: permanent,
		failures,
		message: format!("{placed} chunks placed"),
	}
}
