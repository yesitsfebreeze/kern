use std::sync::Arc;

use parking_lot::RwLock;

use crate::base::locks::{read_recovered, write_recovered};
use crate::base::math::clamp_confidence;
use crate::base::store::FlushOutcome;
use crate::base::types::Source;
use crate::base::util::truncate;

use super::{load_graph, Client, Endpoint};

/// Bound on optimistic-retry rounds for a CLI write that lost a flush race to
/// another writer (a live daemon's tick, a parallel CLI). Each round reloads the
/// committed state and re-applies, so a handful suffices; the cap just prevents an
/// unbounded spin under pathological contention.
const WRITE_RETRIES: u32 = 5;

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_ingest(
	cfg: &crate::config::Config,
	text_parts: Vec<String>,
	file: Option<String>,
	embed_url: &str,
	embed_model: &str,
	reason_url: &str,
	reason_model: &str,
) {
	let (embed_key, reason_key) = (&cfg.embed.key, cfg.reason_key());
	let text = if let Some(path) = file {
		match std::fs::read_to_string(&path) {
			Ok(t) => t,
			Err(e) => {
				eprintln!("read file: {e}");
				return;
			}
		}
	} else {
		text_parts.join(" ")
	};

	if text.is_empty() {
		eprintln!("text or --file required");
		return;
	}

	let g = Arc::new(RwLock::new(load_graph(cfg)));
	let llm_client = Client::new(
		Endpoint::new(reason_url, reason_model, reason_key),
		Endpoint::default(),
		Endpoint::new(embed_url, embed_model, embed_key),
	);
	// No worker save_fn: this command owns persistence so it can use the
	// stale-safe guarded flush below instead of a bare full-snapshot overwrite.
	// One-shot CLI ingest: no tick loop exists here, so question seeding is
	// skipped (no defer hook). Questions are enrichment, not data — the daemon's
	// tick will not backfill them for CLI-ingested entities, which is the
	// documented trade for keeping the worker free of reason-LLM calls.
	let worker = crate::ingest::Worker::new(g.clone(), llm_client, None, None, None);

	let (conf, kind) = clamp_confidence(1.0, "user");
	let src = Source::Inline {
		hash: "user".to_string(),
		section: String::new(),
	};

	// Optimistic concurrency: place the ingest, then flush only if no other writer
	// committed since we loaded. A live daemon (or a parallel CLI) is the second
	// writer the "never two writers on one data_dir" hazard warns about; rather
	// than blindly overwriting — which is exactly how the daemon used to wipe
	// committed ingests — we detect the divergence via the store epoch, reload the
	// committed state, and re-apply. The reload re-embeds, but only on an actual
	// race, which is rare.
	let mut outcome = run_once(&worker, &g, &text, &src, kind, conf, cfg).await;
	for attempt in 0..WRITE_RETRIES {
		// Guard against the epoch observed at LOAD time (flushed_epoch), not a
		// re-read at flush time — otherwise a writer that committed between our load
		// and our flush would go unnoticed and we'd overwrite it.
		let expected = read_recovered(&g).flushed_epoch();
		// Bind before matching: a scrutinee temporary would keep the read guard
		// alive across the whole match — deadlocking write_recovered below and
		// holding the lock across the retry await.
		let flushed = crate::base::persist::flush_guarded(&read_recovered(&g), expected);
		match flushed {
			Ok(FlushOutcome::Flushed { .. }) => break,
			Ok(FlushOutcome::RefusedStale { .. }) if attempt + 1 < WRITE_RETRIES => {
				// Another writer raced ahead; adopt the committed graph (reusing the
				// open store handle — never reopen the env) and re-place.
				{
					let mut w = write_recovered(&g);
					let fresh = super::reload_graph(cfg, &w);
					*w = fresh;
				}
				outcome = run_once(&worker, &g, &text, &src, kind, conf, cfg).await;
			}
			Ok(FlushOutcome::RefusedStale { disk_epoch, expected }) => {
				eprintln!(
					"ingest: persisted under contention after {WRITE_RETRIES} tries \
					 (disk epoch {disk_epoch} vs {expected}); another writer is active on this data_dir"
				);
				break;
			}
			Err(e) => {
				eprintln!("save: {e}");
				break;
			}
		}
	}

	let summary = truncate(&text, 60);
	println!(
		"ingested {summary} (status={} chunks={})",
		outcome.status.as_str(),
		outcome.total_chunks
	);
}

/// One placement pass: run the ingest worker against the (possibly just-reloaded)
/// graph. Factored out so the optimistic-retry loop can re-place after adopting a
/// concurrently-committed graph without duplicating the argument plumbing.
async fn run_once(
	worker: &crate::ingest::Worker,
	_g: &Arc<RwLock<crate::base::graph::GraphGnn>>,
	text: &str,
	src: &Source,
	kind: crate::base::types::EntityKind,
	conf: f64,
	cfg: &crate::config::Config,
) -> crate::ingest::outcome::Outcome {
	worker
		.run(
			text.to_string(),
			src.clone(),
			kind,
			String::new(),
			conf,
			ingest_config(cfg),
		)
		.await
}

/// The ingest `Config` for a CLI ingest: only `dedup_threshold` is carried over
/// from the user's config; every other field uses ingest defaults.
fn ingest_config(cfg: &crate::config::Config) -> crate::ingest::Config {
	crate::ingest::Config {
		dedup_threshold: cfg.ingest.dedup_threshold,
		..Default::default()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ingest_config_carries_dedup_threshold_from_cfg() {
		let mut cfg = crate::config::Config::default();
		cfg.ingest.dedup_threshold = 0.87;
		let ic = ingest_config(&cfg);
		assert_eq!(
			ic.dedup_threshold, 0.87,
			"dedup_threshold comes from the user config"
		);
		// Everything else matches ingest defaults (not the user config).
		assert_eq!(ic.dedup_threshold, 0.87);
		let default_dedup = crate::ingest::Config::default().dedup_threshold;
		assert_ne!(
			0.87, default_dedup,
			"test value differs from the default, so the assertion is meaningful"
		);
	}
}
