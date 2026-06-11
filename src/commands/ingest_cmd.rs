use std::sync::{Arc, RwLock};

use crate::base::locks::read_recovered;
use crate::base::math::clamp_confidence;
use crate::base::types::Source;
use crate::base::util::truncate;

use super::{Client, Endpoint, load_graph, save_graph};

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_ingest(
	cfg: &crate::config::Config,
	text_parts: Vec<String>,
	file: Option<String>,
	no_llm: bool,
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
	let llm_fn: Option<crate::ingest::LlmFunc> = if !no_llm && !reason_url.is_empty() {
		Some(Arc::new(llm_client.complete_func()))
	} else {
		None
	};
	let save_g = g.clone();
	let save_fn: Option<Arc<dyn Fn() + Send + Sync>> = Some(Arc::new(move || {
		let g = read_recovered(&save_g);
		save_graph(&g);
	}));
	let worker = crate::ingest::Worker::new(g.clone(), llm_client, llm_fn, save_fn);

	let (conf, kind) = clamp_confidence(1.0, "user");
	let src = Source::Inline {
		hash: "user".to_string(),
		section: String::new(),
	};

	let outcome = worker
		.run(text.clone(), src, kind, String::new(), conf, ingest_config(cfg))
		.await;
	// No explicit save here: the worker's `save_fn` (above) runs after the job in
	// the worker loop and before `run` returns its outcome, so the graph is
	// already persisted exactly once.

	let summary = truncate(&text, 60);
	println!(
		"ingested {summary} (status={} chunks={})",
		outcome.status.as_str(),
		outcome.total_chunks
	);
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
		assert_eq!(ic.dedup_threshold, 0.87, "dedup_threshold comes from the user config");
		// Everything else matches ingest defaults (not the user config).
		assert_eq!(ic.dedup_threshold, 0.87);
		let default_dedup = crate::ingest::Config::default().dedup_threshold;
		assert_ne!(0.87, default_dedup, "test value differs from the default, so the assertion is meaningful");
	}
}
