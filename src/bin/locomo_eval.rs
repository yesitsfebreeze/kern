//! LoCoMo eval harness (#36): measure kern memory quality.
//!
//! Drives capture→distill→retrieve over the LoCoMo corpus with live ollama
//! models and reports per-category quality (F1 / ROUGE-L / LLM-judge, plus
//! abstention on the adversarial category), retrieved-context size, and query
//! latency. The dataset is CC BY-NC 4.0 — supply it via `--dataset` or
//! `KERN_LOCOMO_PATH`; it is never bundled in the repo.

use clap::Parser;
use kern::bench_support::locomo_run::{run_eval, EvalConfig};
use kern::config::{DEFAULT_ANSWER_MODEL, DEFAULT_EMBED_MODEL, DEFAULT_EMBED_URL};

#[derive(Parser, Debug)]
#[command(name = "locomo_eval", about = "Measure kern memory quality on the LoCoMo benchmark.")]
struct Args {
	/// Path to locomo10.json. Defaults to $KERN_LOCOMO_PATH, then eval/locomo10.json.
	#[arg(long)]
	dataset: Option<String>,
	/// Ollama base URL.
	#[arg(long, default_value = DEFAULT_EMBED_URL)]
	url: String,
	/// Embedding model tag.
	#[arg(long, default_value = DEFAULT_EMBED_MODEL)]
	embed_model: String,
	/// Answerer model tag (kern's reason endpoint glues retrieved context).
	#[arg(long, default_value = DEFAULT_ANSWER_MODEL)]
	answer_model: String,
	/// LLM-judge model tag.
	#[arg(long, default_value = "qwen2.5:7b")]
	judge_model: String,
	/// Limit to the first N dialogues (default: all 10).
	#[arg(long)]
	samples: Option<usize>,
	/// Limit to the first N QA probes per dialogue (default: all).
	#[arg(long)]
	max_qa: Option<usize>,
	/// Dedup cosine threshold at ingest.
	#[arg(long, default_value_t = 0.95)]
	dedup: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();
	let dataset = args
		.dataset
		.or_else(|| std::env::var("KERN_LOCOMO_PATH").ok())
		.unwrap_or_else(|| "eval/locomo10.json".to_string());

	let cfg = EvalConfig {
		dataset_path: dataset.clone(),
		base_url: args.url,
		embed_model: args.embed_model,
		answer_model: args.answer_model,
		judge_model: args.judge_model,
		max_samples: args.samples,
		max_qa_per_sample: args.max_qa,
		dedup_threshold: args.dedup,
	};

	eprintln!("locomo_eval: dataset={dataset} embed={} answer={} judge={}", cfg.embed_model, cfg.answer_model, cfg.judge_model);
	let report = run_eval(&cfg).await?;
	println!("{}", report.summary());
	Ok(())
}
