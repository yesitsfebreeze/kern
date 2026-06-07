use clap::Parser;

use kern::commands::{Cli, dispatch, run_server};
use kern::config::Config;

fn main() {
	use tracing_subscriber::prelude::*;
	let _ = tracing_subscriber::registry()
		.with(journal::JournalTracingLayer::new("kern"))
		.try_init();

	// Floor the worker-thread count. The daemon runs several blocking bridges
	// (tick distillation, ingest embedding, the keepalive ping) that each pin a
	// worker via `block_in_place`/`block_on`. On a low-core box the default
	// (one worker per core) lets those consume every worker, starving the time
	// driver — which freezes the heartbeat AND the watchdog's liveness beat, the
	// exact total stall that wedges the hub. The tick/ingest consumers are serial
	// (≤1 in-flight blocking LLM call each), so ≥4 workers guarantees the async
	// UI/RPC paths and timers always have a thread to run on.
	let workers = std::thread::available_parallelism()
		.map(|n| n.get())
		.unwrap_or(4)
		.max(4);
	let rt = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(workers)
		.enable_all()
		.build()
		.expect("build tokio runtime");

	rt.block_on(async {
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		let cfg = Config::load(&cwd).unwrap_or_default();
		let cli = Cli::parse();

		match cli.command {
			Some(cmd) => dispatch(cmd, &cfg).await,
			None => run_server(&cli, &cfg).await,
		}
	});
}
