use clap::Parser;

use kern::commands::{Cli, dispatch, run_server};
use kern::config::Config;

/// Worker-thread count for the tokio runtime: the detected core count, but never
/// below the hard floor of 4 (and 4 when detection fails). The floor keeps the
/// async UI/RPC/timer paths from being starved by the blocking tick/ingest/
/// keepalive bridges on a low-core box. Pure so the floor logic is unit-tested.
fn worker_thread_count(available: Option<usize>) -> usize {
	available.unwrap_or(4).max(4)
}

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
	let workers = worker_thread_count(std::thread::available_parallelism().map(|n| n.get()).ok());
	let rt = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(workers)
		.enable_all()
		.build()
		.expect("build tokio runtime");

	rt.block_on(async {
		// Pin this instance to its project root (the nearest ancestor holding a
		// `.kern`), so the endpoint tag, data_dir, and capture spool all anchor
		// to the same directory. Without this, a daemon launched from a subdir
		// or the wrong cwd resolves the relative `.kern/data` against an empty
		// location and silently boots an empty graph while still serving queries.
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		let root = Config::resolve_root(&cwd);
		if root != cwd {
			// Operators inspecting where the daemon anchored its data_dir / spool
			// need to see this re-pin; a silent cwd change is hard to diagnose.
			tracing::info!(
				target: "kern",
				from = %cwd.display(),
				to = %root.display(),
				"re-pinned cwd to project root (nearest ancestor with .kern)"
			);
			let _ = std::env::set_current_dir(&root);
		}
		let cfg = Config::load(&root).unwrap_or_default();
		let cli = Cli::parse();

		match cli.command {
			Some(cmd) => dispatch(cmd, &cfg).await,
			None => run_server(&cli, &cfg).await,
		}
	});
}

#[cfg(test)]
mod tests {
	use super::worker_thread_count;

	#[test]
	fn worker_count_honors_the_floor_of_four() {
		// Below the floor (incl. detection failure -> None) clamps up to 4.
		assert_eq!(worker_thread_count(None), 4, "detection failure -> floor");
		assert_eq!(worker_thread_count(Some(1)), 4, "1 core -> floor");
		assert_eq!(worker_thread_count(Some(2)), 4);
		assert_eq!(worker_thread_count(Some(4)), 4, "at the floor");
		// Above the floor passes through.
		assert_eq!(worker_thread_count(Some(8)), 8);
		assert_eq!(worker_thread_count(Some(64)), 64);
	}
}
