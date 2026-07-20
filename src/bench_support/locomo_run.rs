use super::locomo::{self, Sample};
use crate::base::graph::GraphGnn;
use crate::base::types::{EntityKind, Source};
use crate::config::RetrievalConfig;
use crate::ingest::distill;
use crate::ingest::{Config, Worker};
use crate::llm::{Client as LlmClient, Endpoint};
use crate::retrieval::answer;
use crate::retrieval::score::QueryOptions;
use crate::retrieval::seed::Mode;
use crate::types::{EmbedFunc, LlmFunc};
use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;
use std::time::Instant;

// Loss attribution (improvements doc, item 0): Grounded scores the answerer+judge
// ceiling, GroundedRetrieval splits distill loss from retrieval loss.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ContextMode {
	#[default]
	Kern,
	Grounded,
	GroundedRetrieval,
}

impl ContextMode {
	pub fn parse(s: &str) -> Option<Self> {
		match s {
			"kern" => Some(Self::Kern),
			"grounded" => Some(Self::Grounded),
			"grounded-retrieval" => Some(Self::GroundedRetrieval),
			_ => None,
		}
	}
}

// LoCoMo golds are 2–4 words; full-sentence answers handicap token-F1 against
// published short-answer numbers. Eval-only — the product prompt stays untouched.
const SHORT_ANSWER_STYLE: &str = "Answer with only the fact — a few words, not a full sentence.";

// Rendered locomo10 conversations measure 11k-24k tokens; anything smaller
// silently truncates the early sessions and measures recency, not the ceiling.
const GROUNDED_NUM_CTX: u64 = 32768;

const GROUNDED_RETRIEVAL_TOP_K: usize = 10;

#[derive(Clone)]
pub struct EvalConfig {
	pub dataset_path: String,
	pub base_url: String,
	pub answer_url: Option<String>,
	pub judge_url: Option<String>,
	pub embed_model: String,
	pub answer_model: String,
	pub judge_model: String,
	pub max_samples: Option<usize>,
	pub max_qa_per_sample: Option<usize>,
	pub dedup_threshold: f64,
	pub seed: i64,
	pub context_mode: ContextMode,
	// Max concurrent samples (ingest+eval). >1 needs OLLAMA_NUM_PARALLEL on the
	// server; grounded mode multiplies its 32k KV cache per slot — watch VRAM.
	pub concurrency: usize,
	pub min_deliver: f64,
	// Each probe costs up to 3 LLM calls: HyDE expansion, LLM rerank, synthesis.
	// Both extras default on and neither has ever been scored (ROADMAP 3) — these
	// make that measurable instead of assumed.
	pub hyde: bool,
	pub rerank: bool,
	pub probe_log: Option<String>,
	// true = ignore the on-disk claims cache entirely (no read, no write).
	pub fresh_distill: bool,
}

impl ContextMode {
	pub fn name(self) -> &'static str {
		match self {
			Self::Kern => "kern",
			Self::Grounded => "grounded",
			Self::GroundedRetrieval => "grounded-retrieval",
		}
	}
}

const CLAIMS_CACHE_DIR: &str = "eval/cache";

// Cache rows survive prompt/model/seed changes by keying on all three — an
// edited distill prompt hashes to new keys, stale rows just stop being read.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedClaim {
	text: String,
	kind: String,
	valid_from_epoch: Option<u64>,
}

impl From<&distill::Claim> for CachedClaim {
	fn from(c: &distill::Claim) -> Self {
		Self {
			text: c.text.clone(),
			kind: c.descriptor.clone(),
			valid_from_epoch: c.valid_from.and_then(|t| {
				t.duration_since(std::time::UNIX_EPOCH)
					.ok()
					.map(|d| d.as_secs())
			}),
		}
	}
}

impl From<CachedClaim> for distill::Claim {
	fn from(c: CachedClaim) -> Self {
		Self {
			text: c.text,
			descriptor: c.kind,
			valid_from: c
				.valid_from_epoch
				.map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s)),
		}
	}
}

#[derive(serde::Serialize)]
pub struct ProbeRecord {
	pub sample_id: String,
	pub mode: &'static str,
	pub category: u8,
	pub question: String,
	pub gold: Option<String>,
	pub pred: Option<String>,
	pub verdict: Option<bool>,
	pub abstained: Option<bool>,
	pub top_cosine: Option<f64>,
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
	// Per non-adversarial gold: cosine of the nearest ingested claim (distill
	// coverage, improvements doc item 5). Populated only in grounded-retrieval mode.
	pub gold_nearest_cosine: Vec<f64>,
	// Probes dropped from scoring (transport failures) — nonzero means the
	// denominators differ from a clean run.
	pub embed_errors: usize,
	pub answer_errors: usize,
	pub judge_errors: usize,
	// Wall clock per phase, measured at the top level — summing per-sample times
	// would double-count overlap under concurrency. Without these the
	// answer/judge split has to be inferred from summed latencies, which counts
	// queueing as work and misattributes where the time actually goes.
	pub sample_phase_secs: f64,
	pub judge_phase_secs: f64,
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
			gold_nearest_cosine: Vec::new(),
			embed_errors: 0,
			answer_errors: 0,
			judge_errors: 0,
			sample_phase_secs: 0.0,
			judge_phase_secs: 0.0,
		}
	}

	fn merge(&mut self, other: EvalReport) {
		self.total_claims += other.total_claims;
		self.n_samples += other.n_samples;
		self.n_queries += other.n_queries;
		self.ctx_entities_sum += other.ctx_entities_sum;
		self.ctx_chars_sum += other.ctx_chars_sum;
		self.embed_errors += other.embed_errors;
		self.answer_errors += other.answer_errors;
		self.judge_errors += other.judge_errors;
		self.latencies_ms.extend(other.latencies_ms);
		self.gold_nearest_cosine.extend(other.gold_nearest_cosine);
		for (cat, agg) in other.per_category {
			let e = self.per_category.entry(cat).or_default();
			e.n += agg.n;
			e.f1 += agg.f1;
			e.rouge += agg.rouge;
			e.judge_correct += agg.judge_correct;
			e.abstain_correct += agg.abstain_correct;
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
		if self.sample_phase_secs + self.judge_phase_secs > 0.0 {
			let sum_lat: f64 = self.latencies_ms.iter().map(|m| *m as f64).sum::<f64>() / 1000.0;
			out.push_str(&format!(
				"phases: ingest+answer {:.1} min  judge {:.1} min  (summed query latency {:.1} min — exceeds the answer phase when concurrency>1, since each latency includes queue wait)\n",
				self.sample_phase_secs / 60.0,
				self.judge_phase_secs / 60.0,
				sum_lat / 60.0,
			));
		}
		if self.embed_errors + self.answer_errors + self.judge_errors > 0 {
			out.push_str(&format!(
				"errors: embed={} answer={} judge={} (embed/answer probes are excluded from every denominator; judge errors score as incorrect)\n",
				self.embed_errors, self.answer_errors, self.judge_errors,
			));
		}
		if !self.gold_nearest_cosine.is_empty() {
			let mut c = self.gold_nearest_cosine.clone();
			c.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
			let covered = c.iter().filter(|v| **v >= 0.6).count();
			out.push_str(&format!(
				"gold→nearest-claim cosine: p10={:.3} p50={:.3} p90={:.3}  ≥0.6: {:.1}% ({}/{})\n",
				crate::base::util::percentile_sorted(&c, 0.10).unwrap_or(0.0),
				crate::base::util::percentile_sorted(&c, 0.50).unwrap_or(0.0),
				crate::base::util::percentile_sorted(&c, 0.90).unwrap_or(0.0),
				covered as f64 * 100.0 / c.len() as f64,
				covered,
				c.len(),
			));
		}
		out.push('\n');
		out.push_str("category      n     F1   ROUGE-L  judge/abstain\n");
		out.push_str("------------------------------------------------\n");
		let mut tot_n = 0usize;
		let mut tot_correct = 0usize;
		for (cat, a) in &self.per_category {
			let n = a.n.max(1) as f64;
			let (correct, label) = if *cat == locomo::ADVERSARIAL_CATEGORY {
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

fn answer_client(cfg: &EvalConfig) -> LlmClient {
	let answer_url = cfg.answer_url.as_deref().unwrap_or(&cfg.base_url);
	LlmClient::new(
		Endpoint::new(answer_url, &cfg.answer_model, ""),
		Endpoint::default(),
		Endpoint::new(&cfg.base_url, &cfg.embed_model, ""),
	)
	.for_eval(cfg.seed)
}

// Owned handles so a spawned sample task runs without borrowing run_eval's stack.
struct SampleTaskCtx {
	client: LlmClient,
	judge: LlmClient,
	llm: LlmFunc,
	embed_fn: EmbedFunc,
	rcfg: RetrievalConfig,
	icfg: Config,
	mode: ContextMode,
	max_qa_per_sample: Option<usize>,
	probe_concurrency: usize,
	ecfg: EvalConfig,
}

async fn process_one_sample(
	ctx: Arc<SampleTaskCtx>,
	sample: Sample,
	take: usize,
	i: usize,
) -> (usize, EvalReport, Vec<ProbeRecord>) {
	eprintln!("[{}/{}] ingesting {} ...", i + 1, take, sample.sample_id);
	// Fresh graph per dialogue: LoCoMo dialogues are independent personas.
	let graph: Arc<RwLock<GraphGnn>> = Arc::new(RwLock::new(GraphGnn::new()));
	let worker = Worker::new(graph.clone(), ctx.client.clone(), None, None, None);

	// Grounded mode never touches the graph — the full conversation is the context.
	let claims = if ctx.mode == ContextMode::Grounded {
		0
	} else {
		ingest_sample(&worker, &ctx.llm, &sample, &ctx.icfg, &ctx.ecfg).await
	};
	eprintln!(
		"[{}/{}] ingested {claims} claims, running {} QA probes ...",
		i + 1,
		take,
		sample.qa.len()
	);

	let mut report = EvalReport::new();
	report.total_claims = claims;
	report.n_samples = 1;

	let pctx = ProbeCtx {
		client: ctx.client.clone(),
		llm: ctx.llm.clone(),
		embed_fn: ctx.embed_fn.clone(),
		rcfg: ctx.rcfg.clone(),
		mode: ctx.mode,
		graph: graph.clone(),
		full_convo: Arc::new(match ctx.mode {
			ContextMode::Grounded => render_conversation(&sample),
			_ => String::new(),
		}),
	};
	let records = eval_sample(
		&pctx,
		&sample,
		ctx.max_qa_per_sample,
		ctx.probe_concurrency,
		&mut report,
	)
	.await;
	eprintln!("[{}/{}] done ({} queries)", i + 1, take, report.n_queries);
	(i, report, records)
}

pub async fn run_eval(cfg: &EvalConfig) -> Result<EvalReport, String> {
	let samples = locomo::load(&cfg.dataset_path)?;
	let take = cfg.max_samples.unwrap_or(samples.len());

	let judge_url = cfg.judge_url.as_deref().unwrap_or(&cfg.base_url);
	let mut client = answer_client(cfg);
	if cfg.context_mode == ContextMode::Grounded {
		client = client.with_num_ctx(GROUNDED_NUM_CTX);
	}
	// Judge at temperature 0: verdicts must not carry sampling noise.
	let judge = LlmClient::new(
		Endpoint::new(judge_url, &cfg.judge_model, ""),
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
	let rcfg = RetrievalConfig {
		min_deliver_score: cfg.min_deliver,
		hyde_enabled: cfg.hyde,
		rerank_enabled: cfg.rerank,
		..Default::default()
	};
	let icfg = Config {
		dedup_threshold: cfg.dedup_threshold,
		..Default::default()
	};

	// Total LLM load stays at cfg.concurrency: sample-level parallelism provides
	// min(C, take) concurrent samples, each running C/min(C, take) probes at once.
	let c = cfg.concurrency.max(1);
	let probe_concurrency = (c / c.min(take.max(1))).max(1);

	let ctx = Arc::new(SampleTaskCtx {
		client,
		judge,
		llm,
		embed_fn,
		rcfg,
		icfg,
		mode: cfg.context_mode,
		max_qa_per_sample: cfg.max_qa_per_sample,
		probe_concurrency,
		ecfg: cfg.clone(),
	});

	let sem = Arc::new(tokio::sync::Semaphore::new(cfg.concurrency.max(1)));
	let mut set = tokio::task::JoinSet::new();

	for (i, sample) in samples.into_iter().take(take).enumerate() {
		let permit = sem
			.clone()
			.acquire_owned()
			.await
			.map_err(|e| e.to_string())?;
		let ctx = ctx.clone();
		set.spawn(async move {
			let _permit = permit;
			process_one_sample(ctx, sample, take, i).await
		});
	}

	let mut report = EvalReport::new();
	let mut all_records: Vec<(usize, Vec<ProbeRecord>)> = Vec::new();
	let sample_phase_start = Instant::now();
	while let Some(res) = set.join_next().await {
		let (i, sub, records) = res.map_err(|e| e.to_string())?;
		report.merge(sub);
		all_records.push((i, records));
	}
	report.sample_phase_secs = sample_phase_start.elapsed().as_secs_f64();
	// Samples complete out of order; sort so the probe log is byte-reproducible.
	all_records.sort_by_key(|(i, _)| *i);
	let mut records: Vec<ProbeRecord> = all_records.into_iter().flat_map(|(_, r)| r).collect();

	let judge_phase_start = Instant::now();
	judge_all(&ctx.judge, cfg.concurrency.max(1), &mut records, &mut report).await;
	report.judge_phase_secs = judge_phase_start.elapsed().as_secs_f64();

	if let Some(path) = &cfg.probe_log {
		use std::io::Write;
		let mut f = std::fs::OpenOptions::new()
			.create(true)
			.append(true)
			.open(path)
			.map_err(|e| format!("probe log {path}: {e}"))?;
		for r in &records {
			let line = serde_json::to_string(r).map_err(|e| e.to_string())?;
			writeln!(f, "{line}").map_err(|e| e.to_string())?;
		}
	}

	Ok(report)
}

fn distill_locomo(conversation: &str, llm: &dyn Fn(&str) -> String) -> Option<Vec<distill::Claim>> {
	if conversation.trim().is_empty() {
		return Some(Vec::new());
	}
	let raw = llm(&distill_prompt(conversation));
	if raw.trim().is_empty() {
		return None;
	}
	Some(distill::parse_claims(&raw))
}

// Distillation is deterministic per (prompt, model, seed) — cache it so re-runs
// skip ~40 min of LLM calls AND compare modes over byte-identical graphs.
fn distill_cached(
	cfg: &EvalConfig,
	conversation: &str,
	llm: &dyn Fn(&str) -> String,
) -> Option<Vec<distill::Claim>> {
	if cfg.fresh_distill || conversation.trim().is_empty() {
		return distill_locomo(conversation, llm);
	}
	let key = crate::base::util::content_hash(&format!(
		"{}|{}|{}",
		cfg.answer_model,
		cfg.seed,
		distill_prompt(conversation)
	));
	let path = std::path::Path::new(CLAIMS_CACHE_DIR).join(format!("{key}.json"));
	if let Ok(bytes) = std::fs::read(&path) {
		if let Ok(cached) = serde_json::from_slice::<Vec<CachedClaim>>(&bytes) {
			return Some(cached.into_iter().map(Into::into).collect());
		}
	}
	let claims = distill_locomo(conversation, llm)?;
	let rows: Vec<CachedClaim> = claims.iter().map(CachedClaim::from).collect();
	if let Ok(json) = serde_json::to_vec(&rows) {
		let _ = std::fs::create_dir_all(CLAIMS_CACHE_DIR);
		let _ = std::fs::write(&path, json);
	}
	Some(claims)
}

fn distill_prompt(conversation: &str) -> String {
	format!(
		"Extract durable, reusable personal facts from this social dialogue. \
Output ONLY a JSON array. Each element: \
{{\"text\": \"<one self-contained statement>\", \"kind\": \"<one of: preference, \
decision, project, fact, reference, procedural>\"}}.\n\
Rules:\n\
- Dates are first-class. When an event has a specific date, ALWAYS embed it in \
the claim (e.g. \"Caroline attended an LGBTQ support group on 7 May 2023\", \
not \"Caroline attends an LGBTQ support group\").\n\
- The [Session N — <date>] header is the session's real date. Resolve every \
relative date (\"yesterday\", \"last week\", \"last May\") against it and write \
the absolute date in the claim (e.g. \"in May 2022\", never \"last May\").\n\
- Also extract non-dated facts: personality traits, skills, hobbies, job, \
health, relationships, opinions, plans — anything that would help answer \
future questions about this person.\n\
- Each claim is self-contained: include the person's name and full context.\n\
- ONE claim per distinct fact. Skip greetings and filler.\n\
If nothing is worth keeping, output []. No markdown wrapping.\n\n\
DIALOGUE:\n{conversation}\n"
	)
}

fn render_session(session: &locomo::Session) -> String {
	let mut convo = format!("[Session {} — {}]\n", session.index, session.date_time);
	for t in &session.turns {
		convo.push_str(&t.speaker);
		convo.push_str(": ");
		convo.push_str(&t.text);
		convo.push('\n');
	}
	convo
}

fn render_conversation(sample: &Sample) -> String {
	sample
		.sessions
		.iter()
		.map(render_session)
		.collect::<Vec<_>>()
		.join("\n")
}

async fn ingest_sample(
	worker: &Worker,
	llm: &LlmFunc,
	sample: &Sample,
	icfg: &Config,
	cfg: &EvalConfig,
) -> usize {
	let mut total = 0;
	for session in &sample.sessions {
		let convo = render_session(session);
		let claims = match distill_cached(cfg, &convo, llm.as_ref()) {
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

// Owned handles so probe/judge tasks can be tokio::spawn'ed; everything is
// Arc-backed, cloning is pointer-cheap.
#[derive(Clone)]
struct ProbeCtx {
	client: LlmClient,
	llm: LlmFunc,
	embed_fn: EmbedFunc,
	rcfg: RetrievalConfig,
	mode: ContextMode,
	graph: Arc<RwLock<GraphGnn>>,
	full_convo: Arc<String>,
}

struct ProbeOutcome {
	idx: usize,
	q: locomo::QaItem,
	pred: Option<String>,
	latency_ms: Option<u128>,
	ctx_entities: usize,
	ctx_chars: usize,
	gold_cosine: Option<f64>,
	embed_error: bool,
	answer_error: bool,
}

impl ProbeOutcome {
	fn err(idx: usize, q: locomo::QaItem, embed: bool) -> Self {
		Self {
			idx,
			q,
			pred: None,
			latency_ms: None,
			ctx_entities: 0,
			ctx_chars: 0,
			gold_cosine: None,
			embed_error: embed,
			answer_error: !embed,
		}
	}
}

fn eval_answer_instructions(context_name: &str) -> String {
	format!(
		"Answer the question concisely using only the {context_name} above. \
		 Do not restate it. {SHORT_ANSWER_STYLE} \
		 If the {context_name} does not contain the answer, say exactly: {}",
		answer::NO_ANSWER
	)
}

fn grounded_prompt(conversation: &str, question: &str) -> String {
	format!(
		"Full conversation log:\n\n{conversation}\nQuestion: {question}\n{} \
		 When the answer is a date, resolve relative references \
		 (\"yesterday\", \"last week\") against the [Session N — <date>] \
		 headers and give the absolute date.",
		eval_answer_instructions("conversation")
	)
}

fn grounded_retrieval_prompt(facts: &[String], question: &str) -> String {
	let mut prompt = String::from("Relevant facts:\n");
	for (i, f) in facts.iter().enumerate() {
		prompt.push_str(&format!("{}. {}\n", i + 1, f));
	}
	prompt.push_str(&format!(
		"\nQuestion: {question}\n{}",
		eval_answer_instructions("facts")
	));
	prompt
}

async fn run_probe(ctx: ProbeCtx, idx: usize, q: locomo::QaItem) -> ProbeOutcome {
	match ctx.mode {
		ContextMode::Kern => {
			let Ok(qvec) = ctx.client.embed(&q.question).await else {
				return ProbeOutcome::err(idx, q, true);
			};
			let t0 = Instant::now();
			let res = {
				let g = crate::base::locks::read_recovered(&ctx.graph);
				answer::query(
					&g,
					&ctx.rcfg,
					&qvec,
					&q.question,
					Mode::Hybrid,
					Some(&ctx.llm),
					Some(&ctx.embed_fn),
					Some(QueryOptions {
						answer_style: Some(SHORT_ANSWER_STYLE.to_string()),
						..Default::default()
					}),
				)
			};
			let latency = t0.elapsed().as_millis();
			let ctx_chars = res
				.entities
				.iter()
				.map(|e| e.entity.text().len())
				.sum::<usize>();
			ProbeOutcome {
				idx,
				pred: Some(res.answer),
				latency_ms: Some(latency),
				ctx_entities: res.entities.len(),
				ctx_chars,
				gold_cosine: None,
				embed_error: false,
				answer_error: false,
				q,
			}
		}
		ContextMode::Grounded => {
			match ctx
				.client
				.complete(&grounded_prompt(&ctx.full_convo, &q.question))
				.await
			{
				Ok(a) => ProbeOutcome {
					idx,
					pred: Some(a),
					latency_ms: None,
					ctx_entities: 0,
					ctx_chars: 0,
					gold_cosine: None,
					embed_error: false,
					answer_error: false,
					q,
				},
				Err(_) => ProbeOutcome::err(idx, q, false),
			}
		}
		ContextMode::GroundedRetrieval => {
			// Nearest-to-gold claims = the best context retrieval could ever
			// deliver; adversarial probes have no gold, so the question stands in.
			let target = q.answer.as_deref().unwrap_or(&q.question);
			let Ok(gvec) = ctx.client.embed(target).await else {
				return ProbeOutcome::err(idx, q, true);
			};
			let (facts, top_cosine) = {
				let g = crate::base::locks::read_recovered(&ctx.graph);
				let hits =
					crate::base::search::search_all_unlocked(&g, &gvec, GROUNDED_RETRIEVAL_TOP_K);
				let top = hits.first().map(|h| h.score).unwrap_or(0.0);
				let facts: Vec<String> = hits
					.iter()
					.filter_map(|h| {
						crate::base::search::find_entity(&g, &h.entity_id)
							.map(|(e, _)| e.text().to_string())
					})
					.collect();
				(facts, top)
			};
			let gold_cosine = (!q.is_adversarial()).then_some(top_cosine);
			let pred = if facts.is_empty() {
				Ok(answer::NO_ANSWER.to_string())
			} else {
				ctx
					.client
					.complete(&grounded_retrieval_prompt(&facts, &q.question))
					.await
			};
			match pred {
				Ok(a) => ProbeOutcome {
					idx,
					pred: Some(a),
					latency_ms: None,
					ctx_entities: 0,
					ctx_chars: 0,
					gold_cosine,
					embed_error: false,
					answer_error: false,
					q,
				},
				Err(_) => ProbeOutcome {
					gold_cosine,
					..ProbeOutcome::err(idx, q, false)
				},
			}
		}
	}
}

// Two-phase (answer all, then judge all): answerer and judge can't share an 8 GB
// GPU, so interleaving would pay a model reload per probe. Within each phase,
// `concurrency` requests run in flight (Semaphore-capped tokio tasks).
async fn eval_sample(
	pctx: &ProbeCtx,
	sample: &Sample,
	max_qa: Option<usize>,
	concurrency: usize,
	report: &mut EvalReport,
) -> Vec<ProbeRecord> {
	let limit = max_qa.unwrap_or(sample.qa.len());
	let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));

	let handles: Vec<_> = sample
		.qa
		.iter()
		.take(limit)
		.cloned()
		.enumerate()
		.map(|(idx, q)| {
			let ctx = pctx.clone();
			let sem = sem.clone();
			tokio::spawn(async move {
				let _permit = sem.acquire_owned().await.expect("semaphore never closed");
				run_probe(ctx, idx, q).await
			})
		})
		.collect();
	let mut outcomes = Vec::with_capacity(handles.len());
	for h in handles {
		if let Ok(o) = h.await {
			outcomes.push(o);
		}
	}
	outcomes.sort_by_key(|o| o.idx);

	let mode_name = pctx.mode.name();
	let mut records = Vec::with_capacity(outcomes.len());

	for o in outcomes {
		if let Some(c) = o.gold_cosine {
			report.gold_nearest_cosine.push(c);
		}
		let Some(answer_text) = o.pred else {
			report.embed_errors += o.embed_error as usize;
			report.answer_errors += o.answer_error as usize;
			records.push(ProbeRecord {
				sample_id: sample.sample_id.clone(),
				mode: mode_name,
				category: o.q.category,
				question: o.q.question,
				gold: o.q.answer,
				pred: None,
				verdict: None,
				abstained: None,
				top_cosine: o.gold_cosine,
			});
			continue;
		};

		if let Some(l) = o.latency_ms {
			report.latencies_ms.push(l);
		}
		report.n_queries += 1;
		report.ctx_entities_sum += o.ctx_entities;
		report.ctx_chars_sum += o.ctx_chars;

		let pred = answer_text.trim().to_string();
		if std::env::var_os("KERN_EVAL_DEBUG").is_some() {
			eprintln!(
				"  [debug] q={:?}\n    gold={:?}\n    pred={:?}",
				o.q.question,
				o.q.answer.as_deref().unwrap_or("<adversarial>"),
				pred
			);
		}
		let agg = report.per_category.entry(o.q.category).or_default();
		agg.n += 1;

		let mut abstained = None;
		if o.q.is_adversarial() {
			let hit = locomo::is_abstention(&pred);
			abstained = Some(hit);
			if hit {
				agg.abstain_correct += 1;
			}
		} else if let Some(gold) = o.q.answer.as_deref() {
			agg.f1 += locomo::token_f1(&pred, gold);
			agg.rouge += locomo::rouge_l(&pred, gold);
		}
		records.push(ProbeRecord {
			sample_id: sample.sample_id.clone(),
			mode: mode_name,
			category: o.q.category,
			question: o.q.question,
			gold: o.q.answer,
			pred: Some(pred),
			verdict: None,
			abstained,
			top_cosine: o.gold_cosine,
		});
	}

	// Judging is deliberately NOT done here — see judge_all: one global phase
	// after every sample answers, so the judge model loads once per run instead
	// of swapping against the answerer per dialogue.
	records
}

// Records needing a verdict: answered, non-adversarial, gold present.
fn needs_judging(r: &ProbeRecord) -> bool {
	r.category != locomo::ADVERSARIAL_CATEGORY && r.gold.is_some() && r.pred.is_some()
}

// Measured 2026-07-20: judging was 17.7 of 24.9 min wall clock when interleaved
// per dialogue (answerer and a 7B judge swapping VRAM on one 8 GB card). Running
// it once, after all answering, removes the swap entirely.
async fn judge_all(
	judge: &LlmClient,
	concurrency: usize,
	records: &mut [ProbeRecord],
	report: &mut EvalReport,
) {
	let pending: Vec<usize> = (0..records.len()).filter(|i| needs_judging(&records[*i])).collect();
	if pending.is_empty() {
		return;
	}
	eprintln!("judging {} answers ...", pending.len());

	let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
	let handles: Vec<_> = pending
		.into_iter()
		.map(|i| {
			let judge = judge.clone();
			let sem = sem.clone();
			let prompt = locomo::judge_prompt(
				&records[i].question,
				records[i].gold.as_deref().unwrap_or(""),
				records[i].pred.as_deref().unwrap_or(""),
			);
			tokio::spawn(async move {
				let _permit = sem.acquire_owned().await.expect("semaphore never closed");
				let verdict = judge
					.complete(&prompt)
					.await
					.map(|r| locomo::parse_judge_verdict(&r));
				(i, verdict)
			})
		})
		.collect();

	for h in handles {
		let Ok((i, verdict)) = h.await else {
			continue;
		};
		if verdict.is_err() {
			report.judge_errors += 1;
		}
		let correct = verdict.unwrap_or(false);
		records[i].verdict = Some(correct);
		if correct {
			if let Some(agg) = report.per_category.get_mut(&records[i].category) {
				agg.judge_correct += 1;
			}
		}
	}
}

fn hops_within(g: &GraphGnn, from: &str, to: &str, max_hops: usize) -> Option<usize> {
	let mut frontier = vec![from.to_string()];
	let mut seen: std::collections::HashSet<String> = frontier.iter().cloned().collect();
	for depth in 1..=max_hops {
		let mut next = Vec::new();
		for id in &frontier {
			for n in crate::retrieval::expand::neighbor_ids(g, id) {
				if n == to {
					return Some(depth);
				}
				if seen.insert(n.to_string()) {
					next.push(n.to_string());
				}
			}
		}
		if next.is_empty() {
			return None;
		}
		frontier = next;
	}
	None
}

// Improvements doc item 2, experiment 1: before building a deeper walk, count
// whether gold-supporting claims are graph-connected at all. If they are not,
// the fix is ingest-side edges, and deeper expansion has nothing to traverse.
pub async fn run_multihop_paths(cfg: &EvalConfig) -> Result<String, String> {
	const MULTI_HOP: u8 = 1;
	let samples = locomo::load(&cfg.dataset_path)?;
	let take = cfg.max_samples.unwrap_or(samples.len());
	let client = answer_client(cfg);
	let llm: LlmFunc = Arc::new(client.complete_func());
	let icfg = Config {
		dedup_threshold: cfg.dedup_threshold,
		..Default::default()
	};

	let (mut probes, mut pairable, mut linked) = (0usize, 0usize, 0usize);
	let mut out = String::new();
	for (i, sample) in samples.iter().take(take).enumerate() {
		eprintln!("[{}/{}] ingesting {} ...", i + 1, take, sample.sample_id);
		let graph: Arc<RwLock<GraphGnn>> = Arc::new(RwLock::new(GraphGnn::new()));
		let worker = Worker::new(graph.clone(), client.clone(), None, None, None);
		ingest_sample(&worker, &llm, sample, &icfg, cfg).await;

		let limit = cfg.max_qa_per_sample.unwrap_or(sample.qa.len());
		for q in sample
			.qa
			.iter()
			.filter(|q| q.category == MULTI_HOP)
			.take(limit)
		{
			let Some(gold) = q.answer.as_deref() else {
				continue;
			};
			let Ok(gvec) = client.embed(gold).await else {
				continue;
			};
			let g = crate::base::locks::read_recovered(&graph);
			let ids: Vec<String> = crate::base::search::search_all_unlocked(&g, &gvec, 3)
				.into_iter()
				.map(|h| h.entity_id)
				.collect();
			probes += 1;
			if ids.len() < 2 {
				out.push_str(&format!("  <2 nearby claims: {}\n", q.question));
				continue;
			}
			pairable += 1;
			let mut best: Option<usize> = None;
			for a in 0..ids.len() {
				for b in a + 1..ids.len() {
					if let Some(d) = hops_within(&g, &ids[a], &ids[b], 2) {
						best = Some(best.map_or(d, |x| x.min(d)));
					}
				}
			}
			match best {
				Some(d) => {
					linked += 1;
					out.push_str(&format!("  linked ({d} hops): {}\n", q.question));
				}
				None => out.push_str(&format!("  unlinked: {}\n", q.question)),
			}
		}
	}

	out.push_str(&format!(
		"\nmulti-hop probes: {probes}  with >=2 nearby claims: {pairable}  any-pair <=2 hops: {linked}\n\
		 verdict: {}\n",
		if pairable > 0 && linked * 2 >= pairable {
			"paths exist -- a bounded second expansion wave is worth testing"
		} else {
			"edges missing -- fix is ingest-side (entity co-mention linking); a deeper walk has nothing to traverse"
		}
	));
	Ok(out)
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

	// fresh_distill: true — tests must never touch the on-disk claims cache.
	fn test_eval_config() -> EvalConfig {
		EvalConfig {
			dataset_path: String::new(),
			base_url: String::new(),
			answer_url: None,
			judge_url: None,
			embed_model: "embed-model".into(),
			answer_model: "answer-model".into(),
			judge_model: "judge-model".into(),
			max_samples: None,
			max_qa_per_sample: None,
			dedup_threshold: 0.95,
			seed: 0,
			context_mode: ContextMode::Kern,
			concurrency: 1,
			min_deliver: 0.0,
			hyde: true,
			rerank: true,
			probe_log: None,
			fresh_distill: true,
		}
	}

	#[test]
	fn cached_claims_roundtrip_through_the_disk_format() {
		let claim = distill::Claim {
			text: "Alice prefers tea".into(),
			descriptor: "preference".into(),
			valid_from: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(86400)),
		};
		let cached = CachedClaim::from(&claim);
		let json = serde_json::to_vec(&vec![cached]).unwrap();
		let back: Vec<CachedClaim> = serde_json::from_slice(&json).unwrap();
		let restored: distill::Claim = back.into_iter().next().unwrap().into();
		assert_eq!(restored, claim);
	}

	#[test]
	fn fresh_distill_always_calls_the_llm() {
		let cfg = test_eval_config();
		let calls = std::sync::atomic::AtomicUsize::new(0);
		let llm = |_: &str| {
			calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
			r#"[{"text":"a fact","kind":"fact"}]"#.to_string()
		};
		for _ in 0..2 {
			let claims = distill_cached(&cfg, "Bob: hi", &llm).expect("claims");
			assert_eq!(claims.len(), 1);
		}
		assert_eq!(
			calls.load(std::sync::atomic::Ordering::SeqCst),
			2,
			"fresh_distill must never serve from cache"
		);
	}

	fn rec(category: u8, gold: Option<&str>, pred: Option<&str>) -> ProbeRecord {
		ProbeRecord {
			sample_id: "s".into(),
			mode: "kern",
			category,
			question: "q?".into(),
			gold: gold.map(str::to_string),
			pred: pred.map(str::to_string),
			verdict: None,
			abstained: None,
			top_cosine: None,
		}
	}

	#[test]
	fn only_answered_non_adversarial_probes_with_gold_are_judged() {
		assert!(needs_judging(&rec(1, Some("gold"), Some("pred"))));
		assert!(
			!needs_judging(&rec(locomo::ADVERSARIAL_CATEGORY, None, Some("pred"))),
			"adversarial scores on abstention, never the judge"
		);
		assert!(
			!needs_judging(&rec(1, Some("gold"), None)),
			"a dropped probe has no answer to judge"
		);
		assert!(
			!needs_judging(&rec(1, None, Some("pred"))),
			"no gold means nothing to judge against"
		);
	}

	#[test]
	fn summary_shows_phase_timing_only_when_measured() {
		let mut r = EvalReport::new();
		assert!(!r.summary().contains("phases:"));
		r.sample_phase_secs = 600.0;
		r.judge_phase_secs = 120.0;
		let s = r.summary();
		assert!(s.contains("ingest+answer 10.0 min"), "{s}");
		assert!(s.contains("judge 2.0 min"), "{s}");
	}

	#[test]
	fn summary_shows_error_counts_only_when_nonzero() {
		let mut r = EvalReport::new();
		assert!(!r.summary().contains("errors:"));
		r.answer_errors = 2;
		assert!(r.summary().contains("errors: embed=0 answer=2 judge=0"));
	}

	#[test]
	fn context_mode_parses_the_three_ablations_and_rejects_junk() {
		assert_eq!(ContextMode::parse("kern"), Some(ContextMode::Kern));
		assert_eq!(ContextMode::parse("grounded"), Some(ContextMode::Grounded));
		assert_eq!(
			ContextMode::parse("grounded-retrieval"),
			Some(ContextMode::GroundedRetrieval)
		);
		assert_eq!(ContextMode::parse("orcale"), None);
	}

	#[test]
	fn distill_locomo_prompt_instructs_absolute_date_resolution() {
		let llm = |p: &str| {
			assert!(
				p.contains("Resolve every relative date"),
				"date-resolution rule must be in the distill prompt"
			);
			"[]".to_string()
		};
		distill_locomo("Bob: I painted a sunrise last May.", &llm);
	}

	#[test]
	fn grounded_prompts_carry_short_answer_style_and_the_abstention_string() {
		let p = grounded_prompt("[Session 1 — 1 Jan 2024]\nA: hi\n", "when?");
		assert!(p.contains("A: hi"), "conversation inlined");
		assert!(p.contains(SHORT_ANSWER_STYLE));
		assert!(p.contains(answer::NO_ANSWER));

		let p = grounded_retrieval_prompt(&["fact one".into(), "fact two".into()], "what?");
		assert!(p.contains("1. fact one"));
		assert!(p.contains("2. fact two"));
		assert!(p.contains(SHORT_ANSWER_STYLE));
		assert!(p.contains(answer::NO_ANSWER));
	}

	#[test]
	fn summary_shows_gold_coverage_only_when_measured() {
		let mut r = EvalReport::new();
		assert!(!r.summary().contains("gold→nearest-claim"));
		r.gold_nearest_cosine = vec![0.2, 0.5, 0.7, 0.9];
		let s = r.summary();
		assert!(s.contains("gold→nearest-claim"), "coverage line: {s}");
		assert!(s.contains("≥0.6: 50.0% (2/4)"), "coverage share: {s}");
	}

	#[test]
	fn hops_within_walks_two_hops_and_respects_the_bound() {
		use crate::base::reason::add_reason;
		use crate::base::types::{mk_entity, Kern, Reason, ReasonKind};

		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		for (id, text) in [("a", "alpha"), ("b", "beta"), ("c", "gamma"), ("d", "delta")] {
			k.entities
				.insert(id.into(), mk_entity(id, text, 0.0, EntityKind::Claim));
		}
		for (rid, from, to) in [("r1", "a", "b"), ("r2", "b", "c")] {
			add_reason(
				&mut k,
				Reason {
					from: from.into(),
					to: to.into(),
					id: rid.into(),
					text: "links".into(),
					kind: ReasonKind::Similarity,
					..Default::default()
				},
			);
		}
		g.kerns.insert("k".into(), k);

		assert_eq!(hops_within(&g, "a", "b", 2), Some(1));
		assert_eq!(hops_within(&g, "a", "c", 2), Some(2), "walks via b");
		assert_eq!(hops_within(&g, "c", "a", 2), Some(2), "edges are undirected");
		assert_eq!(hops_within(&g, "a", "c", 1), None, "bound respected");
		assert_eq!(hops_within(&g, "a", "d", 2), None, "no path to isolated node");
	}

	#[test]
	fn render_conversation_joins_all_session_headers() {
		let sample = Sample {
			sample_id: "t".into(),
			sessions: vec![
				Session {
					index: 1,
					date_time: "1 Jan 2024".into(),
					turns: vec![Turn {
						speaker: "A".into(),
						dia_id: "d1".into(),
						text: "hi".into(),
					}],
				},
				Session {
					index: 2,
					date_time: "2 Feb 2024".into(),
					turns: Vec::new(),
				},
			],
			qa: Vec::new(),
		};
		let convo = render_conversation(&sample);
		assert!(convo.contains("[Session 1 — 1 Jan 2024]"));
		assert!(convo.contains("[Session 2 — 2 Feb 2024]"));
		assert!(convo.contains("A: hi"));
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
		let ecfg = test_eval_config();

		let claims = ingest_sample(&worker, &llm, &sample, &icfg, &ecfg).await;
		assert_eq!(claims, 1, "the single distilled claim is counted");

		let g = crate::base::locks::read_recovered(&graph);
		let entities: usize = g.all().iter().map(|k| k.entities.len()).sum();
		assert!(
			entities > 0,
			"worker placed at least the claim document into the graph"
		);
	}
}
