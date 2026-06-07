//! Live LoCoMo eval driver (#36).
//!
//! Drives the real kern pipeline end-to-end against ollama: each dialogue's
//! sessions go through capture→distill→ingest (the canonical `Worker`), then
//! every QA probe is answered via `retrieval::answer::query` and scored.
//! Per-category quality (token-F1, ROUGE-L, LLM-judge; abstention for the
//! adversarial category), retrieved-context size as a token-efficiency proxy,
//! and query latency (p50/p95) are aggregated into an [`EvalReport`].
//!
//! All numeric scoring lives in the pure, unit-tested [`super::locomo`] module;
//! this module is the orchestration + I/O around it.

use super::locomo::{self, Sample};
use crate::base::graph::GraphGnn;
use crate::base::types::{EntityKind, Source};
use crate::config::RetrievalConfig;
use crate::ingest::distill::distill;
use crate::ingest::{Config, Worker};
use crate::llm::{Client as LlmClient, Endpoint};
use crate::retrieval::answer;
use crate::retrieval::seed::Mode;
use crate::types::{EmbedFunc, LlmFunc};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Knobs for a run. Models are ollama tags; `base_url` is the ollama endpoint.
pub struct EvalConfig {
	pub dataset_path: String,
	pub base_url: String,
	pub embed_model: String,
	/// The answerer: kern's `reason` endpoint glues retrieved context into prose.
	pub answer_model: String,
	/// The LLM-judge model (separate so it can differ from the answerer).
	pub judge_model: String,
	pub max_samples: Option<usize>,
	pub max_qa_per_sample: Option<usize>,
	pub dedup_threshold: f64,
}

/// Running totals for one LoCoMo category.
#[derive(Default, Clone)]
pub struct CatAgg {
	pub n: usize,
	/// Sum of token-F1 over answerable questions in this category.
	pub f1: f64,
	/// Sum of ROUGE-L over answerable questions.
	pub rouge: f64,
	/// Count of LLM-judge CORRECT verdicts (answerable only).
	pub judge_correct: usize,
	/// Adversarial only: count of correct abstentions.
	pub abstain_correct: usize,
}

/// Aggregated result of a run.
pub struct EvalReport {
	pub per_category: BTreeMap<u8, CatAgg>,
	pub latencies_ms: Vec<u128>,
	pub total_claims: usize,
	pub n_samples: usize,
	/// Sum of delivered-entity counts across queries (context-size proxy).
	pub ctx_entities_sum: usize,
	/// Sum of delivered-entity text lengths in chars (token-efficiency proxy).
	pub ctx_chars_sum: usize,
	pub n_queries: usize,
}

impl EvalReport {
	fn new() -> Self {
		Self {
			per_category: BTreeMap::new(),
			latencies_ms: Vec::new(),
			total_claims: 0,
			n_samples: 0,
			ctx_entities_sum: 0,
			ctx_chars_sum: 0,
			n_queries: 0,
		}
	}

	/// Human-readable summary table.
	pub fn summary(&self) -> String {
		let mut out = String::new();
		out.push_str(&format!(
			"samples: {}  claims ingested: {}  queries: {}\n",
			self.n_samples, self.total_claims, self.n_queries
		));
		let mut lat = self.latencies_ms.clone();
		lat.sort_unstable();
		out.push_str(&format!(
			"latency ms: p50={} p95={} p99={} max={}\n",
			percentile(&lat, 50.0),
			percentile(&lat, 95.0),
			percentile(&lat, 99.0),
			lat.last().copied().unwrap_or(0),
		));
		if self.n_queries > 0 {
			out.push_str(&format!(
				"avg retrieved context: {:.1} entities / {:.0} chars per query (token-efficiency proxy)\n",
				self.ctx_entities_sum as f64 / self.n_queries as f64,
				self.ctx_chars_sum as f64 / self.n_queries as f64,
			));
		}
		out.push('\n');
		out.push_str("category      n     F1   ROUGE-L  judge/abstain\n");
		out.push_str("------------------------------------------------\n");
		let mut tot_n = 0usize;
		let mut tot_correct = 0usize;
		for (cat, a) in &self.per_category {
			let n = a.n.max(1) as f64;
			let (correct, label) = if *cat == 5 {
				(a.abstain_correct, "abstain")
			} else {
				(a.judge_correct, "judge")
			};
			out.push_str(&format!(
				"{:<12} {:>3}  {:>5.3}  {:>6.3}   {:>5.3} ({})\n",
				locomo::category_name(*cat),
				a.n,
				a.f1 / n,
				a.rouge / n,
				correct as f64 / n,
				label,
			));
			tot_n += a.n;
			tot_correct += correct;
		}
		out.push_str("------------------------------------------------\n");
		out.push_str(&format!(
			"overall      {:>3}                   {:>5.3} (judge+abstain)\n",
			tot_n,
			if tot_n == 0 { 0.0 } else { tot_correct as f64 / tot_n as f64 },
		));
		out
	}
}

/// Nearest-rank percentile of an already-sorted slice.
fn percentile(sorted: &[u128], p: f64) -> u128 {
	if sorted.is_empty() {
		return 0;
	}
	let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
	sorted[idx.min(sorted.len() - 1)]
}

/// Run the full eval and return the aggregated report.
pub async fn run_eval(cfg: &EvalConfig) -> Result<EvalReport, String> {
	let samples = locomo::load(&cfg.dataset_path)?;
	let take = cfg.max_samples.unwrap_or(samples.len());

	// Answerer + embedder share one client (reason endpoint = answerer).
	let client = LlmClient::new(
		Endpoint::new(&cfg.base_url, &cfg.answer_model, ""),
		Endpoint::default(),
		Endpoint::new(&cfg.base_url, &cfg.embed_model, ""),
	);
	let judge = LlmClient::new(
		Endpoint::new(&cfg.base_url, &cfg.judge_model, ""),
		Endpoint::default(),
		Endpoint::new(&cfg.base_url, &cfg.embed_model, ""),
	);

	let llm: LlmFunc = Arc::new(client.complete_func());
	let embed_fn: EmbedFunc = {
		let c = client.clone();
		Arc::new(move |t: &str| block_on_embed(&c, t))
	};
	let rcfg = RetrievalConfig::default();
	let icfg = Config { dedup_threshold: cfg.dedup_threshold, ..Default::default() };

	let mut report = EvalReport::new();

	for sample in samples.iter().take(take) {
		// Fresh graph per dialogue: LoCoMo dialogues are independent personas.
		let graph: Arc<RwLock<GraphGnn>> = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), client.clone(), Some(llm.clone()), None);

		report.total_claims += ingest_sample(&worker, &llm, sample, &icfg).await;
		report.n_samples += 1;

		eval_sample(&graph, &client, &judge, &llm, &embed_fn, &rcfg, sample, cfg.max_qa_per_sample, &mut report)
			.await;
	}

	Ok(report)
}

/// Distill each session and ingest every claim through the canonical worker.
/// Returns the number of claims ingested.
async fn ingest_sample(worker: &Worker, llm: &LlmFunc, sample: &Sample, icfg: &Config) -> usize {
	let mut total = 0;
	for session in &sample.sessions {
		let mut convo = format!("[Session {} — {}]\n", session.index, session.date_time);
		for t in &session.turns {
			convo.push_str(&t.speaker);
			convo.push_str(": ");
			convo.push_str(&t.text);
			convo.push('\n');
		}
		let claims = match distill(&convo, llm.as_ref()) {
			Some(c) => c,
			None => continue, // LLM outage on this session; skip
		};
		for c in claims {
			let src = Source::Session {
				session_id: format!("locomo:{}:s{}", sample.sample_id, session.index),
				section: String::new(),
				title: format!("locomo://{}", c.descriptor),
			};
			// The capture spool ingests every distilled claim as `EntityKind::Claim`.
			let _ = worker.run(c.text, src, EntityKind::Claim, c.descriptor, 0.6, icfg.clone()).await;
			total += 1;
		}
	}
	total
}

/// Answer + score every QA probe for one sample.
#[allow(clippy::too_many_arguments)]
async fn eval_sample(
	graph: &Arc<RwLock<GraphGnn>>,
	client: &LlmClient,
	judge: &LlmClient,
	llm: &LlmFunc,
	embed_fn: &EmbedFunc,
	rcfg: &RetrievalConfig,
	sample: &Sample,
	max_qa: Option<usize>,
	report: &mut EvalReport,
) {
	let limit = max_qa.unwrap_or(sample.qa.len());
	for q in sample.qa.iter().take(limit) {
		let qvec = match client.embed(&q.question).await {
			Ok(v) => v,
			Err(_) => continue, // embed outage; skip this probe
		};

		let t0 = Instant::now();
		let res = {
			let g = crate::base::locks::read_recovered(graph);
			answer::query(&g, rcfg, &qvec, &q.question, Mode::Hybrid, Some(llm), Some(embed_fn), None)
		};
		report.latencies_ms.push(t0.elapsed().as_millis());

		report.n_queries += 1;
		report.ctx_entities_sum += res.entities.len();
		report.ctx_chars_sum += res.entities.iter().map(|e| e.entity.text().len()).sum::<usize>();

		let pred = res.answer.trim();
		let agg = report.per_category.entry(q.category).or_default();
		agg.n += 1;

		if q.is_adversarial() {
			if locomo::is_abstention(pred) {
				agg.abstain_correct += 1;
			}
		} else if let Some(gold) = q.answer.as_deref() {
			agg.f1 += locomo::token_f1(pred, gold);
			agg.rouge += locomo::rouge_l(pred, gold);
			let verdict = judge
				.complete(&locomo::judge_prompt(&q.question, gold, pred))
				.await
				.map(|r| locomo::parse_judge_verdict(&r))
				.unwrap_or(false);
			if verdict {
				agg.judge_correct += 1;
			}
		}
	}
}

/// Synchronously resolve an embed call from inside the multi-thread runtime via
/// the shared [`crate::llm::block_on_in_place`] bridge.
fn block_on_embed(client: &LlmClient, text: &str) -> Result<Vec<f64>, String> {
	let client = client.clone();
	let text = text.to_string();
	match crate::llm::block_on_in_place(client.embed(&text)) {
		Some(r) => r.map_err(|e| e.to_string()),
		None => Err("no tokio runtime".into()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn percentile_nearest_rank() {
		let v = [10u128, 20, 30, 40, 50];
		assert_eq!(percentile(&v, 50.0), 30);
		assert_eq!(percentile(&v, 95.0), 50);
		assert_eq!(percentile(&[], 95.0), 0);
	}

	#[test]
	fn summary_runs_on_empty_report() {
		let r = EvalReport::new();
		let s = r.summary();
		assert!(s.contains("category"));
	}
}
