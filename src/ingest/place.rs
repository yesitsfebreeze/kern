use crate::base::accept;
use crate::base::graph::GraphGnn;
use crate::base::types::*;
use crate::base::util;
use crate::crdt::GCounter;
use crate::ingest::dedup::{find_duplicate, update_existing_entity};
use crate::ingest::embed::embed_with_retry;
use crate::ingest::outcome::FailureReport;
use crate::ingest::Job;
use crate::llm::Client as LlmClient;
use std::sync::Arc;

use parking_lot::RwLock;
use std::time::{Duration, SystemTime};

/// Beta-Bernoulli prior params from a clamped `[0,1]` confidence:
/// `Beta(1 + conf, 1 + (1 - conf))`. Single source for the parameterization
/// shared by document- and chunk-entity construction.
fn beta_params_from_confidence(conf: f32) -> (f32, f32) {
	(1.0 + conf, 1.0 + (1.0 - conf))
}

/// Construct an Active entity carrying a single statement, with `confidence`
/// mapped to Beta-Bernoulli params and a fresh creation timestamp.
///
/// This is the ONLY place the document- and chunk-ingest paths materialize an
/// `Entity`, so the ~25 boilerplate default fields live in one spot. That matters
/// beyond DRY: `Entity` is bincode-positional, so two near-identical literals
/// drifting apart (a field added to one but not the other) would silently corrupt
/// every persisted shard. Callers supply only what actually differs.
#[allow(clippy::too_many_arguments)]
fn new_statement_entity(
	id: String,
	text: &str,
	vector: Vec<f32>,
	kind: EntityKind,
	source: Source,
	external_id: String,
	confidence: f64,
	valid_until: Option<SystemTime>,
	unlinked_count: i32,
) -> Entity {
	let conf = confidence.clamp(0.0, 1.0) as f32;
	let (conf_alpha, conf_beta) = beta_params_from_confidence(conf);
	let mut t = Entity {
		id,
		root_id: String::new(),
		external_id,
		superseded_by: String::new(),
		kind,
		status: EntityStatus::Active,
		statements: vec![text.to_string()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector,
		gnn_vector: Vec::new(),
		score: 0.0,
		conf_alpha,
		conf_beta,
		source,
		created_at: Some(SystemTime::now()),
		acl: Acl::default(),
		access_count: GCounter::new(),
		accessed_at: None,
		heat: 0.0,
		heat_updated_at: None,
		updated_at: None,
		valid_until,
		producer_id: String::new(),
		unlinked_count,
		dirty: false,
		// Fresh ingest: world-time validity opens now (falls back to created_at
		// via `valid_from_or_created`), open-ended, never invalidated.
		valid_from: None,
		valid_to: None,
		invalidated_at: None,
	};
	t.refresh_score();
	t
}

pub(crate) async fn place_document(
	graph: &Arc<RwLock<GraphGnn>>,
	embedder: &LlmClient,
	job: &Job,
	doc_id: &str,
	dedup_threshold: f64,
	defer_contradiction: Option<&crate::ingest::worker::DeferContradictionFn>,
) -> (Option<String>, Option<FailureReport>) {
	let vec = match embed_with_retry(embedder, &job.text, "document", 0).await {
		Ok(v) => v,
		Err(fail) => return (None, Some(fail)),
	};

	let (kind, unlinked) = document_kind(job);

	if let Some(existing_id) = find_duplicate(graph, &vec, dedup_threshold) {
		update_existing_entity(
			graph,
			&existing_id,
			&job.text,
			job.confidence,
			kind,
			defer_contradiction,
		);
		return (Some(existing_id), None);
	}


	let external_id = job.source.source_id().unwrap_or_default();
	let valid_until = job
		.config
		.ttl_secs
		.map(|s| SystemTime::now() + Duration::from_secs(s));

	let mut thought = new_statement_entity(
		doc_id.to_string(),
		&job.text,
		vec,
		kind,
		job.source.clone(),
		external_id,
		job.confidence,
		valid_until,
		unlinked,
	);
	// Distilled world-time hint ("since March"), if any — else falls back to
	// created_at via `valid_from_or_created`.
	thought.valid_from = job.config.valid_from;

	let root_id = graph.read().root.id.clone();

	let lex = {
		let mut g = graph.write();
		accept::accept(&mut g, &root_id, thought.clone(), "");
		g.lexical()
	};
	if let Some(lex) = lex {
		lex.insert(&thought.id, &thought.statements.join(" "));
	}

	(Some(doc_id.to_string()), None)
}

pub(crate) fn document_kind(job: &Job) -> (EntityKind, i32) {
	match job.kind {
		EntityKind::Fact => (EntityKind::Fact, -1),
		_ => (EntityKind::Document, 0),
	}
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn place_chunks(
	graph: &Arc<RwLock<GraphGnn>>,
	defer_questions: Option<&crate::ingest::worker::DeferQuestionsFn>,
	defer_contradiction: Option<&crate::ingest::worker::DeferContradictionFn>,
	job: &Job,
	chunks: &[String],
	chunk_vecs: &[Vec<f32>],
	doc_id: &str,
	dedup_threshold: f64,
) -> usize {
	let root_id = graph.read().root.id.clone();

	let mut placed = 0;
	for (i, (chunk, vec)) in chunks.iter().zip(chunk_vecs.iter()).enumerate() {
		if vec.is_empty() {
			continue;
		}

		if let Some(existing_id) = find_duplicate(graph, vec, dedup_threshold) {
			update_existing_entity(
				graph,
				&existing_id,
				chunk,
				job.confidence,
				job.kind,
				defer_contradiction,
			);
			placed += 1;
			continue;
		}

		let external_id = chunk_source_id(&job.source, i);
		let chunk_valid_until = job
			.config
			.ttl_secs
			.map(|s| SystemTime::now() + Duration::from_secs(s));
		let mut thought = build_chunk_entity(
			chunk,
			vec,
			job.kind,
			&job.source,
			&external_id,
			job.confidence,
			chunk_valid_until,
		);
		thought.valid_from = job.config.valid_from;
		let tid = thought.id.clone();
		let joined = thought.statements.join(" ");

		let (result, lex) = {
			let mut g = graph.write();
			let r = accept::accept(&mut g, &root_id, thought, doc_id);
			let l = g.lexical();
			(r, l)
		};
		if let Some(lex) = lex {
			lex.insert(&tid, &joined);
		}

		if !result.deduped {
			// Question seeding is DEFERRED to the tick (`SeedQuestions` task) —
			// it was a blocking reason-LLM call per chunk right here, which made
			// the worker LLM-bound and starved every queued ingest (measured: a
			// one-line sync ingest waited 69.7 minutes). The hook just enqueues.
			if let Some(defer) = defer_questions {
				defer(&result.entity_id);
			}
		}

		placed += 1;
	}
	placed
}

pub fn build_chunk_entity(
	text: &str,
	vec: &[f32],
	kind: EntityKind,
	source: &Source,
	external_id: &str,
	confidence: f64,
	valid_until: Option<SystemTime>,
) -> Entity {
	new_statement_entity(
		util::content_hash(text),
		text,
		vec.to_vec(),
		kind,
		source.clone(),
		external_id.to_string(),
		confidence,
		valid_until,
		0,
	)
}

pub fn chunk_source_id(source: &Source, index: usize) -> String {
	format!("{}#chunk{}", source.section(), index)
}

// Question seeding RELOCATED to `tick::tasks::do_seed_questions` — the worker
// defers it via `DeferQuestionsFn` so the reason-LLM never runs on the ingest
// commit path. One implementation, one owner (the tick).

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ingest::Config;

	fn session_source() -> Source {
		Source::Session {
			session_id: "s".into(),
			section: "sec".into(),
			title: String::new(),
		}
	}

	fn job(text: &str, confidence: f64) -> Job {
		Job {
			text: text.into(),
			source: session_source(),
			kind: EntityKind::Claim,
			descriptor: String::new(),
			confidence,
			config: Config::default(),
			result_tx: None,
		}
	}

	fn empty_graph() -> Arc<RwLock<GraphGnn>> {
		Arc::new(RwLock::new(GraphGnn::new()))
	}

	/// Total entities across every kern. `accept` routes new thoughts off the
	/// root dispatcher into a spawned generic child, so a root-only count would
	/// miss them — count graph-wide.
	fn total_entity_count(g: &Arc<RwLock<GraphGnn>>) -> usize {
		let gg = g.read();
		gg.all().iter().map(|k| k.entities.len()).sum()
	}

	#[test]
	fn beta_params_map_confidence_to_prior() {
		assert_eq!(beta_params_from_confidence(1.0), (2.0, 1.0));
		assert_eq!(beta_params_from_confidence(0.0), (1.0, 2.0));
		assert_eq!(beta_params_from_confidence(0.5), (1.5, 1.5));
	}

	#[test]
	fn chunk_source_id_is_section_scoped() {
		assert_eq!(chunk_source_id(&session_source(), 3), "sec#chunk3");
	}

	#[test]
	fn build_chunk_entity_carries_text_vector_and_confidence() {
		let e = build_chunk_entity(
			"hello world",
			&[0.1, 0.2, 0.3],
			EntityKind::Claim,
			&session_source(),
			"sec#chunk0",
			1.0,
			None,
		);
		assert_eq!(
			e.id,
			util::content_hash("hello world"),
			"id is the content hash"
		);
		assert_eq!(e.statements, vec!["hello world".to_string()]);
		assert_eq!(e.vector, vec![0.1, 0.2, 0.3]);
		assert_eq!(e.external_id, "sec#chunk0");
		assert_eq!(e.unlinked_count, 0);
		assert!(matches!(e.kind, EntityKind::Claim));
		assert!(matches!(e.status, EntityStatus::Active));
		assert_eq!(e.chunks.len(), 1, "single statement-ref chunk part");
		// confidence 1.0 -> Beta(2, 1)
		assert_eq!((e.conf_alpha, e.conf_beta), (2.0, 1.0));
	}

	#[test]
	fn build_chunk_entity_clamps_out_of_range_confidence() {
		// Above 1.0 clamps to 1.0 -> Beta(2,1); below 0 clamps to 0 -> Beta(1,2).
		let hi = build_chunk_entity(
			"x",
			&[1.0],
			EntityKind::Claim,
			&session_source(),
			"e",
			5.0,
			None,
		);
		assert_eq!((hi.conf_alpha, hi.conf_beta), (2.0, 1.0));
		let lo = build_chunk_entity(
			"y",
			&[1.0],
			EntityKind::Claim,
			&session_source(),
			"e",
			-3.0,
			None,
		);
		assert_eq!((lo.conf_alpha, lo.conf_beta), (1.0, 2.0));
	}

	#[test]
	fn place_chunks_inserts_each_distinct_nonempty_chunk() {
		let g = empty_graph();
		let chunks = vec!["alpha beta".to_string(), "gamma delta".to_string()];
		// Orthogonal vectors so neither chunk dedups against the other.
		let vecs = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
		let placed = place_chunks(&g, None, None, &job("doc", 1.0), &chunks, &vecs, "doc1", 0.95);
		assert_eq!(placed, 2, "both distinct chunks placed");
		assert_eq!(
			total_entity_count(&g),
			2,
			"both accepted into the root kern"
		);
	}

	#[test]
	fn place_chunks_skips_empty_vectors() {
		let g = empty_graph();
		let chunks = vec!["a".to_string(), "b".to_string()];
		// First chunk failed to embed (empty vec) — it must be skipped, not placed.
		let vecs = vec![Vec::new(), vec![1.0, 0.0]];
		let placed = place_chunks(&g, None, None, &job("doc", 1.0), &chunks, &vecs, "doc1", 0.95);
		assert_eq!(placed, 1, "only the chunk with a real vector is placed");
		assert_eq!(total_entity_count(&g), 1);
	}

	#[test]
	fn place_chunks_defers_question_seeding_via_the_hook() {
		// Question seeding moved to the tick: instead of a blocking reason-LLM
		// call per chunk, place_chunks hands each freshly placed entity id to
		// the defer hook (which the daemon wires to enqueue a SeedQuestions
		// tick task). The hook must fire once per placed, non-deduped chunk.
		use std::sync::Mutex;
		let g = empty_graph();
		let chunks = vec!["the sky is blue".to_string()];
		let vecs = vec![vec![1.0, 0.0, 0.0]];
		let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
		let seen_c = seen.clone();
		let defer: crate::ingest::worker::DeferQuestionsFn =
			Arc::new(move |id: &str| seen_c.lock().unwrap().push(id.to_string()));

		let placed = place_chunks(
			&g,
			Some(&defer),
			None,
			&job("doc", 1.0),
			&chunks,
			&vecs,
			"doc1",
			0.95,
		);
		assert_eq!(placed, 1);

		let ids = seen.lock().unwrap();
		assert_eq!(ids.len(), 1, "one defer per placed chunk");
		assert!(!ids[0].is_empty(), "hook receives the placed entity id");
	}

	#[tokio::test]
	async fn place_document_reports_failure_and_leaves_graph_untouched_on_embed_error() {
		let g = empty_graph();
		// Dead loopback endpoint: every embed attempt fails, so place_document must
		// bail with a FailureReport before mutating the graph.
		let embedder = LlmClient::new_embed_only("http://127.0.0.1:1", "test");
		let (id, fail) = place_document(&g, &embedder, &job("a document", 1.0), "doc1", 0.95, None).await;
		assert!(
			id.is_none(),
			"no entity id is returned when embedding fails"
		);
		assert!(fail.is_some(), "a failure report is surfaced");
		assert_eq!(
			total_entity_count(&g),
			0,
			"graph is untouched on embed failure"
		);
	}
}
