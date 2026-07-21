use std::sync::Arc;

use parking_lot::RwLock;

use crate::base::math::clamp_confidence;
use crate::base::store::FlushOutcome;
use crate::base::types::Source;
use crate::base::util::truncate;

use super::{load_graph, Client, Endpoint};

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
		Endpoint::new(embed_url, embed_model, embed_key),
	);
	let worker = crate::ingest::Worker::new(g.clone(), llm_client, None, None, None);

	let (conf, kind) = clamp_confidence(1.0, "user");
	// Identity per ingest, not a shared constant: a constant hash made every
	// CLI ingest the same source, so each one superseded the previous fact.
	let src = Source::Inline {
		hash: crate::base::util::content_hash(&text),
		section: String::new(),
	};

	let mut outcome = run_once(&worker, &g, &text, &src, kind, conf, cfg).await;
	for attempt in 0..WRITE_RETRIES {
		// Guard against the epoch observed at LOAD time, not a re-read at flush time —
		// else a writer that committed in between gets overwritten unseen.
		let expected = g.read().flushed_epoch();
		// Bind before matching: a scrutinee temporary keeps the read guard alive
		// across the match — deadlocking the write() below.
		let flushed = crate::base::persist::flush_guarded(&g.read(), expected);
		match flushed {
			Ok(FlushOutcome::Flushed { .. }) => break,
			Ok(FlushOutcome::RefusedStale { .. }) if attempt + 1 < WRITE_RETRIES => {
				// Adopt the committed graph reusing the open store handle — never reopen the env.
				{
					let mut w = g.write();
					let fresh = super::reload_graph(cfg, &w);
					*w = fresh;
				}
				outcome = run_once(&worker, &g, &text, &src, kind, conf, cfg).await;
			}
			Ok(FlushOutcome::RefusedStale {
				disk_epoch,
				expected,
			}) => {
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
	for f in &outcome.failures {
		eprintln!(
			"  {} #{} ({}): {}",
			f.scope, f.chunk_index, f.class, f.error
		);
	}
}

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
		assert_eq!(ic.dedup_threshold, 0.87);
		let default_dedup = crate::ingest::Config::default().dedup_threshold;
		assert_ne!(
			0.87, default_dedup,
			"test value differs from the default, so the assertion is meaningful"
		);
	}
}
