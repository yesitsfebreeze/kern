use super::locomo::{self, Sample};
use crate::base::graph::GraphGnn;
use crate::base::types::{EntityKind, Source};
use crate::config::RetrievalConfig;
use crate::ingest::distill;
use crate::ingest::{Config, Worker};
use crate::llm::{Client as LlmClient, Endpoint};
use crate::retrieval::answer;
use crate::retrieval::seed::Mode;
use crate::types::{EmbedFunc, LlmFunc};
use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;
use std::time::Instant;

pub struct EvalConfig {
	pub dataset_path: String,
	pub base_url: String,
	pub embed_model: String,
	pub answer_model: String,
	pub judge_model: String,
	pub max_samples: Option<usize>,
	pub max_qa_per_sample: Option<usize>,
	pub dedup_threshold: f64,
	pub seed: i64,
}

#[derive(Default, Clone, serde::Serialize)]
pub struct CatAgg {
	pub n: usize,
	pub f1: f64,
	pub rouge: f64,
	pub judge_correct: usize,
	pub abstain_correct: usize,
}

#[derive(serde::Serialize)]
pub struct EvalReport {
	pub per_category: BTreeMap<u8, CatAgg>,
	pub latencies_ms: Vec<u128>,
	pub total_claims: usize,
	pub n_samples: usize,
	pub ctx_entities_sum: usize,
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
			crate::base::util::percentile_sorted(&lat, 0.50).unwrap_or(0),
			crate::base::util::percentile_sorted(&lat, 0.95).unwrap_or(0),
			crate::base::util::percentile_sorted(&lat, 0.99).unwrap_or(0),
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
			if tot_n == 0 {
				0.0
			} else {
				tot_correct as f64 / tot_n as f64
			},
		));
		out
	}
}

pub async fn run_eval(cfg: &EvalConfig) -> Result<EvalReport, String> {
	let samples = locomo::load(&cfg.dataset_path)?;
	let take = cfg.max_samples.unwrap_or(samples.len());

	let client = LlmClient::new(
		Endpoint::new(&cfg.base_url, &cfg.answer_model, ""),
		Endpoint::default(),
		Endpoint::new(&cfg.base_url, &cfg.embed_model, ""),
	)
	.for_eval(cfg.seed);
	// Judge at temperature 0: verdicts must not carry sampling noise.
	let judge = LlmClient::new(
		Endpoint::new(&cfg.base_url, &cfg.judge_model, ""),
		Endpoint::default(),
		Endpoint::new(&cfg.base_url, &cfg.embed_model, ""),
	)
	.for_eval(cfg.seed)
	.with_temperature(0.0);

	let llm: LlmFunc = Arc::new(client.complete_func());
	let embed_fn: EmbedFunc = {
		let c = client.clone();
		Arc::new(move |t: &str| block_on_embed(&c, t))
	};
	let rcfg = RetrievalConfig::default();
	let icfg = Config {
		dedup_threshold: cfg.dedup_threshold,
		..Default::default()
	};

	let eval_ctx = EvalContext {
		client: &client,
		judge: &judge,
		llm: &llm,
		embed_fn: &embed_fn,
		rcfg: &rcfg,
	};

	let mut report = EvalReport::new();

	for (i, sample) in samples.iter().take(take).enumerate() {
		eprintln!("[{}/{}] ingesting {} ...", i + 1, take, sample.sample_id);
		// Fresh graph per dialogue: LoCoMo dialogues are independent personas.
		let graph: Arc<RwLock<GraphGnn>> = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), client.clone(), None, None, None);

		let claims = ingest_sample(&worker, &llm, sample, &icfg).await;
		eprintln!(
			"[{}/{}] ingested {claims} claims, running {} QA probes ...",
			i + 1,
			take,
			sample.qa.len()
		);
		report.total_claims += claims;
		report.n_samples += 1;

		eval_sample(
			&eval_ctx,
			&graph,
			sample,
			cfg.max_qa_per_sample,
			&mut report,
		)
		.await;
		eprintln!(
			"[{}/{}] done (total queries so far: {})",
			i + 1,
			take,
			report.n_queries
		);
	}

	Ok(report)
}

fn distill_locomo(conversation: &str, llm: &dyn Fn(&str) -> String) -> Option<Vec<distill::Claim>> {
	if conversation.trim().is_empty() {
		return Some(Vec::new());
	}
	let prompt = format!(
		"Extract durable, reusable personal facts from this social dialogue. \
Output ONLY a JSON array. Each element: \
{{\"text\": \"<one self-contained statement>\", \"kind\": \"<one of: preference, \
decision, project, fact, reference, procedural>\"}}.\n\
Rules:\n\
- Dates are first-class. When an event has a specific date, ALWAYS embed it in \
the claim (e.g. \"Caroline attended an LGBTQ support group on 7 May 2023\", \
not \"Caroline attends an LGBTQ support group\").\n\
- Also extract non-dated facts: personality traits, skills, hobbies, job, \
health, relationships, opinions, plans — anything that would help answer \
future questions about this person.\n\
- Each claim is self-contained: include the person's name and full context.\n\
- ONE claim per distinct fact. Skip greetings and filler.\n\
If nothing is worth keeping, output []. No markdown wrapping.\n\n\
DIALOGUE:\n{conversation}\n"
	);
	let raw = llm(&prompt);
	if raw.trim().is_empty() {
		return None;
	}
	Some(distill::parse_claims(&raw))
}

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
		let claims = match distill_locomo(&convo, llm.as_ref()) {
			Some(c) => c,
			None => continue,
		};
		for c in claims {
			let src = Source::Session {
				session_id: format!("locomo:{}:s{}", sample.sample_id, session.index),
				section: String::new(),
				title: format!("locomo://{}", c.descriptor),
			};
			let _ = worker
				.run(
					c.text,
					src,
					EntityKind::Claim,
					c.descriptor,
					0.6,
					icfg.clone(),
				)
				.await;
			total += 1;
		}
	}
	total
}

struct EvalContext<'a> {
	client: &'a LlmClient,
	judge: &'a LlmClient,
	llm: &'a LlmFunc,
	embed_fn: &'a EmbedFunc,
	rcfg: &'a RetrievalConfig,
}

// Two-phase (answer all, then judge all): answerer and judge can't share an 8 GB
// GPU, so interleaving would pay a model reload per probe.
async fn eval_sample(
	ctx: &EvalContext<'_>,
	graph: &Arc<RwLock<GraphGnn>>,
	sample: &Sample,
	max_qa: Option<usize>,
	report: &mut EvalReport,
) {
	let limit = max_qa.unwrap_or(sample.qa.len());
	let mut to_judge: Vec<(&str, &str, String, u8)> = Vec::new();

	for q in sample.qa.iter().take(limit) {
		let qvec = match ctx.client.embed(&q.question).await {
			Ok(v) => v,
			Err(_) => continue,
		};

		let t0 = Instant::now();
		let res = {
			let g = crate::base::locks::read_recovered(graph);
			answer::query(
				&g,
				ctx.rcfg,
				&qvec,
				&q.question,
				Mode::Hybrid,
				Some(ctx.llm),
				Some(ctx.embed_fn),
				None,
			)
		};
		report.latencies_ms.push(t0.elapsed().as_millis());

		report.n_queries += 1;
		report.ctx_entities_sum += res.entities.len();
		report.ctx_chars_sum += res
			.entities
			.iter()
			.map(|e| e.entity.text().len())
			.sum::<usize>();

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
			to_judge.push((&q.question, gold, pred.to_string(), q.category));
		}
	}

	if !to_judge.is_empty() {
		eprintln!("  judging {} answers ...", to_judge.len());
	}
	for (question, gold, pred, category) in &to_judge {
		let verdict = ctx
			.judge
			.complete(&locomo::judge_prompt(question, gold, pred))
			.await
			.map(|r| locomo::parse_judge_verdict(&r))
			.unwrap_or(false);
		if verdict {
			if let Some(agg) = report.per_category.get_mut(category) {
				agg.judge_correct += 1;
			}
		}
	}
}

fn block_on_embed(client: &LlmClient, text: &str) -> Result<Vec<f32>, String> {
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
		assert_eq!(crate::base::util::percentile_sorted(&v, 0.50), Some(30));
		assert_eq!(crate::base::util::percentile_sorted(&v, 0.95), Some(50));
		assert_eq!(
			crate::base::util::percentile_sorted::<u128>(&[], 0.95),
			None
		);
	}

	#[test]
	fn summary_runs_on_empty_report() {
		let r = EvalReport::new();
		let s = r.summary();
		assert!(s.contains("category"));
	}

	#[test]
	fn summary_with_data_shows_category_rows() {
		let mut r = EvalReport::new();
		r.n_samples = 2;
		r.n_queries = 4;
		r.total_claims = 20;
		r.latencies_ms = vec![10, 20, 80, 120];
		let agg = CatAgg {
			n: 4,
			f1: 3.2,
			rouge: 2.8,
			judge_correct: 3,
			..Default::default()
		};
		r.per_category.insert(0, agg);
		let s = r.summary();
		assert!(s.contains("samples: 2"), "samples in header");
		assert!(s.contains("claims ingested: 20"), "claims in header");
		assert!(s.contains("avg retrieved context"), "ctx proxy row present");
	}

	#[test]
	fn distill_locomo_empty_conversation_returns_empty_vec() {
		let llm = |_: &str| panic!("LLM should not be called for empty input");
		assert_eq!(distill_locomo("", &llm), Some(Vec::new()));
		assert_eq!(distill_locomo("   \n\t  ", &llm), Some(Vec::new()));
	}

	#[test]
	fn distill_locomo_llm_outage_returns_none() {
		let llm = |_: &str| String::new();
		assert_eq!(distill_locomo("Alice: Hi there!", &llm), None);
	}

	#[test]
	fn distill_locomo_valid_json_returns_claims() {
		let llm = |_: &str| {
			r#"[{"text":"Alice prefers tea over coffee","kind":"preference"},{"text":"Alice is a software engineer","kind":"fact"}]"#.to_string()
		};
		let claims = distill_locomo("Alice: I prefer tea.", &llm).expect("claims");
		assert_eq!(claims.len(), 2);
		assert_eq!(claims[0].text, "Alice prefers tea over coffee");
		assert_eq!(claims[0].descriptor, "preference");
		assert_eq!(claims[1].descriptor, "fact");
	}

	#[test]
	fn distill_locomo_malformed_json_returns_empty_claims() {
		let llm = |_: &str| "not json at all".to_string();
		let claims = distill_locomo("Alice: Hi.", &llm).expect("Some result");
		assert!(
			claims.is_empty(),
			"malformed JSON produces no claims, not outage"
		);
	}

	#[test]
	fn distill_locomo_prompt_includes_dialogue_text() {
		let llm = |p: &str| {
			assert!(
				p.contains("Bob: I love Rust."),
				"dialogue text must be in prompt"
			);
			assert!(p.contains("DIALOGUE:"), "DIALOGUE marker must be in prompt");
			"[]".to_string()
		};
		distill_locomo("Bob: I love Rust.", &llm);
	}

	use super::locomo::{Session, Turn};

	async fn serve_embed() -> String {
		let app = axum::Router::new().route(
			"/api/embed",
			axum::routing::post(|_b: axum::Json<serde_json::Value>| async move {
				axum::Json(serde_json::json!({ "embeddings": [[0.1, 0.2, 0.3]] }))
			}),
		);
		crate::test_support::spawn_http(app).await.0
	}

	#[tokio::test]
	async fn ingest_sample_distills_and_flows_claims_through_the_worker() {
		let embed_url = serve_embed().await;
		let embedder = LlmClient::new_embed_only(&embed_url, "embed-model");

		let llm: LlmFunc = Arc::new(|p: &str| {
			if p.contains("DIALOGUE:") {
				r#"[{"text":"Alice prefers tea over coffee","kind":"preference"}]"#.to_string()
			} else {
				String::new()
			}
		});

		let graph: Arc<RwLock<GraphGnn>> = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), embedder, None, None, None);

		let sample = Sample {
			sample_id: "t1".into(),
			sessions: vec![Session {
				index: 1,
				date_time: "1 Jan 2024".into(),
				turns: vec![Turn {
					speaker: "Alice".into(),
					dia_id: "d1".into(),
					text: "I prefer tea.".into(),
				}],
			}],
			qa: Vec::new(),
		};
		let icfg = Config {
			dedup_threshold: 0.95,
			..Default::default()
		};

		let claims = ingest_sample(&worker, &llm, &sample, &icfg).await;
		assert_eq!(claims, 1, "the single distilled claim is counted");

		let g = crate::base::locks::read_recovered(&graph);
		let entities: usize = g.all().iter().map(|k| k.entities.len()).sum();
		assert!(
			entities > 0,
			"worker placed at least the claim document into the graph"
		);
	}
}
