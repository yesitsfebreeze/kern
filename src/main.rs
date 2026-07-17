use clap::Parser;

use kern::commands::{dispatch, run_server, Cli, Commands};
use kern::config::Config;

fn worker_thread_count(available: Option<usize>) -> usize {
	available.unwrap_or(4).max(4)
}

fn main() {
	use tracing_subscriber::prelude::*;
	let _ = tracing_subscriber::registry().try_init();

	// Floor of 4: the blocking bridges (tick distill, ingest embed, keepalive)
	// each pin a worker; fewer workers starves the time driver and wedges the hub.
	let workers = worker_thread_count(std::thread::available_parallelism().map(|n| n.get()).ok());
	let rt = tokio::runtime::Builder::new_multi_thread()
		.worker_threads(workers)
		.enable_all()
		.build()
		.expect("build tokio runtime");

	rt.block_on(async {
		// Pin to the project root (nearest ancestor with `.kern`): a subdir launch
		// would boot an empty graph while still serving queries.
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		let root = Config::resolve_root(&cwd);
		if root != cwd {
			tracing::info!(
				target: "kern",
				from = %cwd.display(),
				to = %root.display(),
				"re-pinned cwd to project root (nearest ancestor with .kern)"
			);
			let _ = std::env::set_current_dir(&root);
		}
		let cfg = Config::load(&root).unwrap_or_default();
		if let Err(e) = cfg.validate() {
			tracing::warn!(
				target: "kern.config",
				error = %e,
				"loaded config failed validation; behaviour may be degraded — fix or remove the offending value"
			);
		}
		let cli = Cli::parse();

		match cli.command {
			Some(Commands::Daemon) => run_server(&cli, &cfg).await,
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
		assert_eq!(worker_thread_count(None), 4, "detection failure -> floor");
		assert_eq!(worker_thread_count(Some(1)), 4, "1 core -> floor");
		assert_eq!(worker_thread_count(Some(2)), 4);
		assert_eq!(worker_thread_count(Some(4)), 4, "at the floor");
		assert_eq!(worker_thread_count(Some(8)), 8);
		assert_eq!(worker_thread_count(Some(64)), 64);
	}
}
