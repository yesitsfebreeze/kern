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

pub(crate) struct Job {
	pub(crate) text: String,
	pub(crate) source: Source,
	pub(crate) kind: EntityKind,
	pub(crate) descriptor: String,
	pub(crate) confidence: f64,
	pub(crate) config: Config,
	pub(crate) result_tx: Option<oneshot::Sender<Outcome>>,
}

/// Post-placement hook: called with each freshly placed (non-deduped) entity
/// id so question seeding can be DEFERRED to the tick instead of running a
/// blocking reason-LLM call on the commit path. The hook must be cheap (it
/// enqueues a `SeedQuestions` task); the worker stays embed-bound.
pub type DeferQuestionsFn = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Worker {
	tx: mpsc::Sender<Job>,
}

impl Worker {
	/// The worker deliberately takes NO reason-LLM: every LLM-bound step
	/// (question seeding, enrichment, naming) is deferred to the tick via
	/// `defer_questions`, and chunk splitting always uses the heuristic
	/// splitter. This is a type-level guarantee that the ingest commit path
	/// is embed-bound — measured before the change: a one-line sync ingest
	/// queued 69.7 minutes behind LLM-bound jobs (600s timeout each).
	pub fn new(
		graph: Arc<RwLock<GraphGnn>>,
		embedder: LlmClient,
		defer_questions: Option<DeferQuestionsFn>,
		save_fn: Option<Arc<dyn Fn() + Send + Sync>>,
	) -> Self {
		let (tx, rx) = mpsc::channel(64);
		tokio::spawn(run_loop(graph, embedder, defer_questions, save_fn, rx));
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
				vec![FailureReport::document_permanent(format!(
					"send failed: {e}"
				))],
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
	defer_questions: Option<DeferQuestionsFn>,
	save_fn: Option<Arc<dyn Fn() + Send + Sync>>,
	mut rx: mpsc::Receiver<Job>,
) {
	while let Some(job) = rx.recv().await {
		let outcome = process(&graph, &embedder, &defer_questions, &job).await;
		// Every job's outcome is journaled (the tracing layer feeds the
		// journal), fire-and-forget jobs included. Before this, an `enqueue`d
		// job that failed vanished without a trace: result_tx is None on that
		// path, so the Outcome — including its FailureReports — was dropped
		// unread. Observed live: 8/8 MCP ingests acked "queued" with nothing
		// in the graph and nothing in the journal to say why.
		log_outcome(&outcome);
		if let Some(sf) = &save_fn {
			sf();
		}
		if let Some(tx) = job.result_tx {
			let _ = tx.send(outcome);
		}
	}
}

/// Severity bucket for an ingest outcome: failures are loud, success is quiet.
/// Pure so the mapping is unit-testable; `log_outcome` applies it.
fn outcome_log_severity(o: &Outcome) -> &'static str {
	match o.status {
		OutcomeStatus::Failed => "error",
		OutcomeStatus::Partial => "warn",
		OutcomeStatus::Committed | OutcomeStatus::Deduped => "info",
	}
}

/// Journal one processed job. The first failure's class+error is inlined so a
/// single journal line answers "what happened to doc X" without a debugger.
fn log_outcome(o: &Outcome) {
	let first_failure = o
		.failures
		.first()
		.map(|f| format!("{}/{}: {}", f.scope, f.class, f.error))
		.unwrap_or_default();
	match outcome_log_severity(o) {
		"error" => tracing::error!(
			target: "kern.ingest",
			doc_id = %o.doc_id,
			status = o.status.as_str(),
			total = o.total_chunks,
			embedded = o.embedded_chunks,
			failed = o.failed_chunks,
			first_failure = %first_failure,
			"ingest job failed"
		),
		"warn" => tracing::warn!(
			target: "kern.ingest",
			doc_id = %o.doc_id,
			status = o.status.as_str(),
			total = o.total_chunks,
			embedded = o.embedded_chunks,
			failed = o.failed_chunks,
			first_failure = %first_failure,
			"ingest job partially committed"
		),
		_ => tracing::info!(
			target: "kern.ingest",
			doc_id = %o.doc_id,
			status = o.status.as_str(),
			total = o.total_chunks,
			embedded = o.embedded_chunks,
			"ingest job committed"
		),
	}
}

/// Resolve the outcome's document identity after placement. `place_document`
/// returns the SURVIVING entity id: the content hash on a fresh place, or an
/// existing entity's id on a dedup merge. The outcome must carry the surviving
/// id — the caller was acked the content hash, and after a merge that id does
/// not exist in the graph — and a `Committed` merge becomes `Deduped` so the
/// two are distinguishable. Partial/Failed are never upgraded.
fn finalize_doc_identity(
	content_id: &str,
	surviving_id: String,
	status: OutcomeStatus,
) -> (String, OutcomeStatus) {
	let deduped = surviving_id != content_id;
	let status = if deduped && status == OutcomeStatus::Committed {
		OutcomeStatus::Deduped
	} else {
		status
	};
	(surviving_id, status)
}

async fn process(
	graph: &Arc<RwLock<GraphGnn>>,
	embedder: &LlmClient,
	defer_questions: &Option<DeferQuestionsFn>,
	job: &Job,
) -> Outcome {
	let doc_id = util::content_hash(&job.text);

	// Heuristic splitting ONLY — the LLM splitter was one reason-LLM call per
	// document on the commit path. The heuristic fallback was already the
	// common case (any LLM hiccup) and keeps the worker embed-bound.
	let chunks = split::split(&job.text, &job.descriptor, None);

	let (doc_thought, doc_fail) =
		place_document(graph, embedder, job, &doc_id, job.config.dedup_threshold).await;
	let Some(surviving_id) = doc_thought else {
		let fail = doc_fail.unwrap_or_else(|| FailureReport::document_permanent("unknown"));
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
	};

	let (chunk_vecs, failures) = embed_chunks(embedder, &chunks).await;

	let placed = place_chunks(
		graph,
		defer_questions.as_ref(),
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

	let status = classify_status(embedded_chunks, failed_chunks);
	// Carry the SURVIVING id (existing entity on a dedup merge) and mark the
	// merge as Deduped — the acked content hash does not exist in the graph
	// after a merge, which previously made merges look like silent loss.
	let (doc_id, status) = finalize_doc_identity(&doc_id, surviving_id, status);

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

/// Classify an ingest outcome from chunk tallies: every chunk embedded ->
/// `Committed`; at least one but not all -> `Partial`; none embedded (and at
/// least one failed) -> `Failed`. Note a zero-chunk document (`failed_chunks == 0`)
/// is `Committed` — the document entity itself was placed.
fn classify_status(embedded_chunks: usize, failed_chunks: usize) -> OutcomeStatus {
	if failed_chunks == 0 {
		OutcomeStatus::Committed
	} else if embedded_chunks > 0 {
		OutcomeStatus::Partial
	} else {
		OutcomeStatus::Failed
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classify_status_maps_tallies_to_outcome() {
		// All chunks embedded -> Committed.
		assert_eq!(classify_status(3, 0), OutcomeStatus::Committed);
		// Some embedded, some failed -> Partial.
		assert_eq!(classify_status(2, 1), OutcomeStatus::Partial);
		// None embedded, some failed -> Failed.
		assert_eq!(classify_status(0, 3), OutcomeStatus::Failed);
		// Zero-chunk document (nothing failed) -> Committed.
		assert_eq!(classify_status(0, 0), OutcomeStatus::Committed);
	}

	#[test]
	fn document_permanent_failure_has_canonical_shape() {
		let f = FailureReport::document_permanent("graph lock poisoned");
		assert_eq!(f.scope, "document");
		assert_eq!(f.chunk_index, 0);
		assert_eq!(f.class, "permanent");
		assert_eq!(f.error, "graph lock poisoned");
	}

	fn session_source() -> Source {
		Source::Session {
			session_id: "s".into(),
			section: "sec".into(),
			title: String::new(),
		}
	}

	fn dead_worker(graph: Arc<RwLock<GraphGnn>>) -> Worker {
		// Embed endpoint that always fails, so the ingest pipeline exercises its
		// failure assembly without needing a live model.
		let embedder = LlmClient::new_embed_only("http://127.0.0.1:1", "test");
		Worker::new(graph, embedder, None, None)
	}

	#[tokio::test]
	async fn enqueue_returns_the_content_hash_doc_id() {
		// The fire-and-forget path hands the caller a doc id immediately; it must be
		// the content hash of the text (the same id the worker will commit under).
		let worker = dead_worker(Arc::new(RwLock::new(GraphGnn::new())));
		let text = "some document text".to_string();
		let doc_id = worker.enqueue(
			text.clone(),
			session_source(),
			EntityKind::Claim,
			String::new(),
			1.0,
			Config::default(),
		);
		assert_eq!(doc_id, util::content_hash(&text));
	}

	fn outcome_with(status: OutcomeStatus) -> Outcome {
		Outcome {
			status,
			doc_id: "d".into(),
			total_chunks: 1,
			embedded_chunks: 1,
			failed_chunks: 0,
			transient_failures: 0,
			permanent_failures: 0,
			failures: Vec::new(),
			message: String::new(),
		}
	}

	#[test]
	fn finalize_doc_identity_marks_dedup_and_keeps_surviving_id() {
		// Dedup merge: place_document returned an EXISTING entity id, not the
		// content hash — the outcome must carry the surviving id and say so,
		// otherwise the caller holds a doc_id that does not exist in the graph
		// (indistinguishable from silent loss).
		let (id, st) =
			finalize_doc_identity("hash-a", "existing-b".to_string(), OutcomeStatus::Committed);
		assert_eq!(id, "existing-b");
		assert_eq!(st, OutcomeStatus::Deduped);

		// Fresh placement: surviving id IS the content hash — status unchanged.
		let (id, st) = finalize_doc_identity("hash-a", "hash-a".to_string(), OutcomeStatus::Committed);
		assert_eq!(id, "hash-a");
		assert_eq!(st, OutcomeStatus::Committed);

		// Partial/Failed outcomes are never upgraded to Deduped — chunk failures
		// still need to surface as such.
		let (_, st) = finalize_doc_identity("hash-a", "existing-b".to_string(), OutcomeStatus::Partial);
		assert_eq!(st, OutcomeStatus::Partial);
	}

	#[test]
	fn outcome_log_severity_maps_status_to_level() {
		// Every processed job must be observable: failures loudly, success quietly.
		assert_eq!(
			outcome_log_severity(&outcome_with(OutcomeStatus::Committed)),
			"info"
		);
		assert_eq!(
			outcome_log_severity(&outcome_with(OutcomeStatus::Deduped)),
			"info"
		);
		assert_eq!(
			outcome_log_severity(&outcome_with(OutcomeStatus::Partial)),
			"warn"
		);
		assert_eq!(
			outcome_log_severity(&outcome_with(OutcomeStatus::Failed)),
			"error"
		);
	}

	#[tokio::test]
	async fn run_assembles_a_failed_outcome_when_document_embedding_fails() {
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = dead_worker(graph.clone());
		let text = "a document that cannot be embedded".to_string();
		let outcome = worker
			.run(
				text.clone(),
				session_source(),
				EntityKind::Claim,
				String::new(),
				1.0,
				Config::default(),
			)
			.await;

		assert_eq!(outcome.status, OutcomeStatus::Failed);
		assert_eq!(
			outcome.doc_id,
			util::content_hash(&text),
			"doc id is the content hash"
		);
		assert!(
			outcome.total_chunks >= 1,
			"non-empty text splits into at least one chunk"
		);
		assert_eq!(
			outcome.failed_chunks, outcome.total_chunks,
			"all chunks counted as failed"
		);
		assert_eq!(outcome.embedded_chunks, 0);
		assert_eq!(
			outcome.failures.len(),
			1,
			"one document-level failure recorded"
		);
		assert_eq!(
			outcome.transient_failures + outcome.permanent_failures,
			1,
			"the single failure is classified exactly once",
		);
		assert_eq!(outcome.message, "document embedding failed");

		// The pipeline mutated nothing — no entity was placed on the failure path.
		let g = graph.read().unwrap();
		assert_eq!(g.all().iter().map(|k| k.entities.len()).sum::<usize>(), 0);
	}
}
