use clap::Parser;
use kern::bench_support::locomo_run::{
	compare_probes, run_eval, run_multihop_paths, ContextMode, EvalConfig,
};
use kern::config::{DEFAULT_ANSWER_MODEL, DEFAULT_EMBED_MODEL, DEFAULT_EMBED_URL};

#[derive(Parser, Debug)]
#[command(
	name = "locomo_eval",
	about = "Measure kern memory quality on the LoCoMo benchmark."
)]
struct Args {
	/// Path to locomo10.json (CC BY-NC 4.0; supply it yourself, never bundled).
	/// Falls back to $KERN_LOCOMO_PATH, then eval/locomo10.json.
	#[arg(long)]
	dataset: Option<String>,
	/// Ollama base URL.
	#[arg(long, default_value = DEFAULT_EMBED_URL)]
	url: String,
	/// Answerer base URL override (e.g. a vLLM `/v1` endpoint); defaults to --url.
	#[arg(long)]
	answer_url: Option<String>,
	/// Judge base URL override; defaults to --url.
	#[arg(long)]
	judge_url: Option<String>,
	/// Embedding model tag.
	#[arg(long, default_value = DEFAULT_EMBED_MODEL)]
	embed_model: String,
	/// Answerer model tag (kern's `reason` endpoint glues retrieved context).
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
	/// Sampling seed forwarded to ollama; vary across runs for error bars.
	#[arg(long, default_value_t = 0)]
	seed: i64,
	/// Ablation: kern (full pipeline) | grounded (answer from the full
	/// conversation, kern skipped) | grounded-retrieval (top-10 claims nearest
	/// the gold answer — splits distill loss from retrieval loss).
	#[arg(long, default_value = "kern")]
	context_mode: String,
	/// Diagnostic instead of an eval: ingest, then report whether the claims
	/// nearest each multi-hop gold share any graph path within 2 hops.
	#[arg(long)]
	multihop_paths: bool,
	/// Concurrent samples (ingest+eval). >1 needs OLLAMA_NUM_PARALLEL on the
	/// server; grounded mode multiplies its 32k KV cache per slot (VRAM).
	#[arg(long, default_value_t = 8)]
	concurrency: usize,
	/// Retrieval delivery floor: results scoring below are dropped, and empty
	/// delivery triggers the abstention gate. 0.0 = baseline-compatible.
	#[arg(long, default_value_t = 0.0)]
	min_deliver: f64,
	/// Disable HyDE query expansion (saves one LLM call per probe).
	#[arg(long)]
	no_hyde: bool,
	/// Disable LLM reranking (saves one LLM call per probe).
	#[arg(long)]
	no_rerank: bool,
	/// Append one JSON line per probe (question, gold, pred, verdict,
	/// top_cosine) — the judge-calibration / coverage-calibration artifact.
	#[arg(long)]
	probe_log: Option<String>,
	/// Ignore the distilled-claims cache (eval/cache/) — no read, no write.
	#[arg(long)]
	fresh_distill: bool,
	/// Compare two probe logs instead of running an eval: paired McNemar over
	/// the probes both runs answered. Use this to judge an A/B, not the CIs.
	#[arg(long, num_args = 2, value_names = ["A.jsonl", "B.jsonl"])]
	compare_probes: Option<Vec<String>>,
	/// Emit a machine-readable (CI-diffable) report instead of the human table.
	#[arg(long)]
	json: bool,
	/// Write the report to a file instead of stdout.
	#[arg(long)]
	output: Option<String>,
}

fn resolve_dataset(arg: Option<String>, env: Option<String>) -> String {
	arg
		.or(env)
		.unwrap_or_else(|| "eval/locomo10.json".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args = Args::parse();

	// Comparing two finished runs needs no dataset, no models, no GPU.
	if let Some(paths) = &args.compare_probes {
		print!("{}", compare_probes(&paths[0], &paths[1])?);
		return Ok(());
	}

	let dataset = resolve_dataset(args.dataset, std::env::var("KERN_LOCOMO_PATH").ok());

	if !std::path::Path::new(&dataset).exists() {
		eprintln!("locomo_eval: dataset not found at `{dataset}`.");
		eprintln!(
			"  Supply it via --dataset <path> or the KERN_LOCOMO_PATH env var \
			 (LoCoMo is CC BY-NC 4.0 and is never bundled in the repo)."
		);
		return Err(format!("dataset not found: {dataset}").into());
	}

	let Some(context_mode) = ContextMode::parse(&args.context_mode) else {
		return Err(format!(
			"unknown --context-mode `{}` (expected kern | grounded | grounded-retrieval)",
			args.context_mode
		)
		.into());
	};

	let cfg = EvalConfig {
		dataset_path: dataset.clone(),
		base_url: args.url,
		answer_url: args.answer_url,
		judge_url: args.judge_url,
		embed_model: args.embed_model,
		answer_model: args.answer_model,
		judge_model: args.judge_model,
		max_samples: args.samples,
		max_qa_per_sample: args.max_qa,
		dedup_threshold: args.dedup,
		seed: args.seed,
		context_mode,
		concurrency: args.concurrency,
		min_deliver: args.min_deliver,
		hyde: !args.no_hyde,
		rerank: !args.no_rerank,
		probe_log: args.probe_log,
		fresh_distill: args.fresh_distill,
	};

	eprintln!(
		"locomo_eval: dataset={dataset} embed={} answer={} judge={} seed={} mode={}",
		cfg.embed_model, cfg.answer_model, cfg.judge_model, cfg.seed, args.context_mode
	);

	if args.multihop_paths {
		let table = run_multihop_paths(&cfg).await?;
		match &args.output {
			Some(path) => std::fs::write(path, &table)?,
			None => println!("{table}"),
		}
		return Ok(());
	}

	let report = run_eval(&cfg).await?;

	let body = if args.json {
		serde_json::to_string_pretty(&report)?
	} else {
		report.summary()
	};
	match &args.output {
		Some(path) => {
			std::fs::write(path, &body)?;
			eprintln!("locomo_eval: wrote report to {path}");
		}
		None => println!("{body}"),
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::resolve_dataset;

	#[test]
	fn dataset_resolution_precedence() {
		assert_eq!(
			resolve_dataset(Some("a.json".into()), Some("b.json".into())),
			"a.json",
			"--dataset wins over the env var",
		);
		assert_eq!(
			resolve_dataset(None, Some("b.json".into())),
			"b.json",
			"env var is used when --dataset is absent",
		);
		assert_eq!(
			resolve_dataset(None, None),
			"eval/locomo10.json",
			"falls back to the default path",
		);
	}
}
