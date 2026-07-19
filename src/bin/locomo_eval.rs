use clap::Parser;
use kern::bench_support::locomo_run::{run_eval, EvalConfig};
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
	let dataset = resolve_dataset(args.dataset, std::env::var("KERN_LOCOMO_PATH").ok());

	if !std::path::Path::new(&dataset).exists() {
		eprintln!("locomo_eval: dataset not found at `{dataset}`.");
		eprintln!(
			"  Supply it via --dataset <path> or the KERN_LOCOMO_PATH env var \
			 (LoCoMo is CC BY-NC 4.0 and is never bundled in the repo)."
		);
		return Err(format!("dataset not found: {dataset}").into());
	}

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
	};

	eprintln!(
		"locomo_eval: dataset={dataset} embed={} answer={} judge={} seed={}",
		cfg.embed_model, cfg.answer_model, cfg.judge_model, cfg.seed
	);
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
