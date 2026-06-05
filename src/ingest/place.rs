use crate::base::accept;
use crate::base::graph::GraphGnn;
use crate::base::types::*;
use crate::base::{math, util};
use crate::crdt::GCounter;
use crate::ingest::dedup::{find_duplicate, update_existing_entity};
use crate::ingest::embed::embed_with_retry;
use crate::ingest::outcome::FailureReport;
use crate::ingest::worker::Job;
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
		update_existing_entity(graph, &existing_id, &job.text, vec, job.confidence);
		return (Some(existing_id), None);
	}

	let (kind, unlinked) = document_kind(job);

	let external_id = job.source.source_id().unwrap_or_default();
	let valid_until = job
		.config
		.ttl_secs
		.map(|s| SystemTime::now() + Duration::from_secs(s));

	let conf = job.confidence.clamp(0.0, 1.0) as f32;
	let (conf_alpha, conf_beta) = beta_params_from_confidence(conf);
	let mut thought = Entity {
		id: doc_id.to_string(),
		root_id: String::new(),
		external_id,
		superseded_by: String::new(),
		kind,
		status: EntityStatus::Active,
		statements: vec![job.text.clone()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector: vec,
		gnn_vector: Vec::new(),
		score: 0.0,
		conf_alpha,
		conf_beta,
		source: job.source.clone(),
		created_at: Some(SystemTime::now()),
		acl: Acl::default(),
		access_count: GCounter::new(),
		accessed_at: None,
		heat: 0.0,
		heat_updated_at: None,
		updated_at: None,
		valid_until,
		producer_id: String::new(),
		unlinked_count: unlinked,
	};
	thought.refresh_score();

	let root_id = match graph.read() {
		Ok(g) => g.root.id.clone(),
		Err(e) => {
			return (
				None,
				Some(FailureReport {
					scope: "document".into(),
					chunk_index: 0,
					class: "permanent".into(),
					error: format!("graph lock poisoned: {e}"),
				}),
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
				Some(FailureReport {
					scope: "document".into(),
					chunk_index: 0,
					class: "permanent".into(),
					error: format!("graph lock poisoned: {e}"),
				}),
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
			update_existing_entity(graph, &existing_id, chunk, vec.clone(), job.confidence);
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
let conf = confidence.clamp(0.0, 1.0) as f32;
	let (alpha, beta) = beta_params_from_confidence(conf);
	let mut t = Entity {
		id: util::content_hash(text),
		root_id: String::new(),
		external_id: external_id.to_string(),
		superseded_by: String::new(),
		kind,
		status: EntityStatus::Active,
		statements: vec![text.to_string()],
		chunks: vec![ChunkPart {
			kind: ChunkPartKind::StatementRef,
			text: String::new(),
			index: 0,
		}],
		vector: vec.to_vec(),
		gnn_vector: Vec::new(),
		score: 0.0,
		conf_alpha: alpha,
		conf_beta: beta,
		source: source.clone(),
		created_at: Some(SystemTime::now()),
		acl: Acl::default(),
		access_count: GCounter::new(),
		accessed_at: None,
		heat: 0.0,
		heat_updated_at: None,
		updated_at: None,
		valid_until,
		producer_id: String::new(),
		unlinked_count: 0,
	};
	t.refresh_score();
	t
}

pub fn chunk_source_id(source: &Source, index: usize) -> String {
	format!("{}#chunk{}", source.section(), index)
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

	let root_id = match graph.read() {
		Ok(g) => g.root.id.clone(),
		Err(_) => return,
	};

	let mut g = match graph.write() {
		Ok(g) => g,
		Err(_) => return,
	};
	for q in questions {
		let rid = math::reason_id(&result.entity_id, "", ReasonKind::Question, q, "");
		let reason = Reason {
			id: rid,
			from: result.entity_id.clone(),
			to: String::new(),
			to_kern_id: String::new(),
			to_net_id: String::new(),
			kind: ReasonKind::Question,
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
