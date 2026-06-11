use crate::base::accept;
use crate::base::graph::GraphGnn;
use crate::base::types::*;
use crate::base::{math, util};
use crate::crdt::GCounter;
use crate::ingest::dedup::{find_duplicate, update_existing_entity};
use crate::ingest::embed::embed_with_retry;
use crate::ingest::outcome::FailureReport;
use crate::ingest::Job;
use crate::types::LlmFunc;
use crate::llm::Client as LlmClient;
use std::sync::{Arc, RwLock};
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
	vector: Vec<f64>,
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
) -> (Option<String>, Option<FailureReport>) {
let vec = match embed_with_retry(embedder, &job.text, "document", 0).await {
		Ok(v) => v,
		Err(fail) => return (None, Some(fail)),
	};

	if let Some(existing_id) = find_duplicate(graph, &vec, dedup_threshold) {
		update_existing_entity(graph, &existing_id, &job.text, job.confidence);
		return (Some(existing_id), None);
	}

	let (kind, unlinked) = document_kind(job);

	let external_id = job.source.source_id().unwrap_or_default();
	let valid_until = job
		.config
		.ttl_secs
		.map(|s| SystemTime::now() + Duration::from_secs(s));

	let thought = new_statement_entity(
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

	let root_id = match graph.read() {
		Ok(g) => g.root.id.clone(),
		Err(e) => {
			return (
				None,
				Some(FailureReport::document_permanent(format!("graph lock poisoned: {e}"))),
			)
		}
	};

	let lex = match graph.write() {
		Ok(mut g) => {
			accept::accept(&mut g, &root_id, thought.clone(), "");
			g.lexical()
		}
		Err(e) => {
			return (
				None,
				Some(FailureReport::document_permanent(format!("graph lock poisoned: {e}"))),
			)
		}
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

pub(crate) fn place_chunks(
	graph: &Arc<RwLock<GraphGnn>>,
	llm: &Option<LlmFunc>,
	job: &Job,
	chunks: &[String],
	chunk_vecs: &[Vec<f64>],
	doc_id: &str,
	dedup_threshold: f64,
) -> usize {
let root_id = match graph.read() {
		Ok(g) => g.root.id.clone(),
		Err(_) => return 0,
	};

	let mut placed = 0;
	for (i, (chunk, vec)) in chunks.iter().zip(chunk_vecs.iter()).enumerate() {
		if vec.is_empty() {
			continue;
		}

		if let Some(existing_id) = find_duplicate(graph, vec, dedup_threshold) {
			update_existing_entity(graph, &existing_id, chunk, job.confidence);
			placed += 1;
			continue;
		}

		let external_id = chunk_source_id(&job.source, i);
		let chunk_valid_until = job
			.config
			.ttl_secs
			.map(|s| SystemTime::now() + Duration::from_secs(s));
		let thought = build_chunk_entity(
			chunk,
			vec,
			job.kind,
			&job.source,
			&external_id,
			job.confidence,
			chunk_valid_until,
		);
		let tid = thought.id.clone();
		let joined = thought.statements.join(" ");

		let (result, lex) = match graph.write() {
			Ok(mut g) => {
				let r = accept::accept(&mut g, &root_id, thought, doc_id);
				let l = g.lexical();
				(r, l)
			}
			Err(_) => continue,
		};
		if let Some(lex) = lex {
			lex.insert(&tid, &joined);
		}

		if !result.deduped {
			if let Some(ref llm_fn) = llm {
				generate_questions(graph, llm_fn, &result, chunk);
			}
		}

		placed += 1;
	}
	placed
}

pub fn build_chunk_entity(
	text: &str,
	vec: &[f64],
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ingest::Config;

	fn session_source() -> Source {
		Source::Session { session_id: "s".into(), section: "sec".into(), title: String::new() }
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
		let gg = g.read().unwrap();
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
		assert_eq!(e.id, util::content_hash("hello world"), "id is the content hash");
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
		let hi = build_chunk_entity("x", &[1.0], EntityKind::Claim, &session_source(), "e", 5.0, None);
		assert_eq!((hi.conf_alpha, hi.conf_beta), (2.0, 1.0));
		let lo = build_chunk_entity("y", &[1.0], EntityKind::Claim, &session_source(), "e", -3.0, None);
		assert_eq!((lo.conf_alpha, lo.conf_beta), (1.0, 2.0));
	}

	#[test]
	fn place_chunks_inserts_each_distinct_nonempty_chunk() {
		let g = empty_graph();
		let chunks = vec!["alpha beta".to_string(), "gamma delta".to_string()];
		// Orthogonal vectors so neither chunk dedups against the other.
		let vecs = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
		let placed = place_chunks(&g, &None, &job("doc", 1.0), &chunks, &vecs, "doc1", 0.95);
		assert_eq!(placed, 2, "both distinct chunks placed");
		assert_eq!(total_entity_count(&g), 2, "both accepted into the root kern");
	}

	#[test]
	fn place_chunks_skips_empty_vectors() {
		let g = empty_graph();
		let chunks = vec!["a".to_string(), "b".to_string()];
		// First chunk failed to embed (empty vec) — it must be skipped, not placed.
		let vecs = vec![Vec::new(), vec![1.0, 0.0]];
		let placed = place_chunks(&g, &None, &job("doc", 1.0), &chunks, &vecs, "doc1", 0.95);
		assert_eq!(placed, 1, "only the chunk with a real vector is placed");
		assert_eq!(total_entity_count(&g), 1);
	}

	#[test]
	fn place_chunks_generates_question_edges_when_llm_present() {
		let g = empty_graph();
		let chunks = vec!["the sky is blue".to_string()];
		let vecs = vec![vec![1.0, 0.0, 0.0]];
		// Stub LLM returns two non-empty lines -> two Question edges off the new entity.
		let llm: Option<LlmFunc> =
			Some(Arc::new(|_: &str| "why is the sky blue?\nwhat color is the sky?".to_string()));
		let placed = place_chunks(&g, &llm, &job("doc", 1.0), &chunks, &vecs, "doc1", 0.95);
		assert_eq!(placed, 1);

		let gg = g.read().unwrap();
		let root = gg.kerns.get(&gg.root.id).unwrap();
		let questions = root
			.reasons
			.values()
			.filter(|r| matches!(r.kind, ReasonKind::Question))
			.count();
		assert_eq!(questions, 2, "two question edges from the 2-line LLM response");
	}

	#[tokio::test]
	async fn place_document_reports_failure_and_leaves_graph_untouched_on_embed_error() {
		let g = empty_graph();
		// Dead loopback endpoint: every embed attempt fails, so place_document must
		// bail with a FailureReport before mutating the graph.
		let embedder = LlmClient::new_embed_only("http://127.0.0.1:1", "test");
		let (id, fail) = place_document(&g, &embedder, &job("a document", 1.0), "doc1", 0.95).await;
		assert!(id.is_none(), "no entity id is returned when embedding fails");
		assert!(fail.is_some(), "a failure report is surfaced");
		assert_eq!(total_entity_count(&g), 0, "graph is untouched on embed failure");
	}
}

pub(crate) fn generate_questions(
	graph: &Arc<RwLock<GraphGnn>>,
	llm_fn: &LlmFunc,
	result: &accept::AcceptResult,
	chunk_text: &str,
) {
	let prompt = format!(
		"Given this knowledge chunk, generate up to 3 questions that this chunk answers. \
		 One question per line. No numbering.\n\n{chunk_text}"
	);
	let response = llm_fn(&prompt);
	if response.is_empty() {
		return;
	}

	let questions: Vec<&str> = response
		.lines()
		.map(|l| l.trim())
		.filter(|l| !l.is_empty())
		.take(3)
		.collect();

	// Single write acquisition: read the root id from the same guard we mutate
	// under, so there is no TOCTOU window between picking the root and editing it
	// (and one fewer lock round-trip).
	let mut g = match graph.write() {
		Ok(g) => g,
		Err(_) => return,
	};
	let root_id = g.root.id.clone();
	for q in questions {
		let rid = math::reason_id(&result.entity_id, "", ReasonKind::Question, q, "");
		let reason = Reason {
			id: rid,
			from: result.entity_id.clone(),
			to: String::new(),
			to_kern_id: String::new(),
			to_net_id: String::new(),
			kind: ReasonKind::Question,
			dirty: false,
			text: q.to_string(),
			vector: Vec::new(),
			score: 0.5,
			traversal_count: GCounter::new(),
			producer_id: String::new(),
		};
		if let Some(kern) = g.get_mut(&root_id) {
			crate::base::reason::add_reason(kern, reason);
		}
	}
}
