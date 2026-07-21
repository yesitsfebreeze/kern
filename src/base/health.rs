use crate::base::graph::GraphGnn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStats {
	pub kerns: usize,
	pub entities: usize,
	pub reasons: usize,
	pub unnamed: usize,
	pub gravitons: Vec<String>,
	// Cold rows dropped by the FIFO cap since this process opened the store. A
	// dropped non-durable entity is unrecoverable, so the count is its only trace.
	pub cold_evicted: u64,
	pub embed_model: String,
	pub embed_dim: usize,
	pub embed_mismatch: bool,
	// Queries dropped by the dimension guard since this process opened. Nonzero
	// with embed_mismatch false means something upstream embeds off-model.
	pub query_dim_rejected: u64,
	// Deliveries that bypassed `min_deliver_score` because nothing cleared it —
	// a degraded answer the caller cannot distinguish from a confident one.
	pub below_floor_deliveries: u64,
	// Entities GC could not age because their timestamp is in the future.
	// Nonzero means compaction is stalled on a clock problem, not on policy.
	pub clock_skew_skips: u64,
	// Chunks dropped because embedding them failed — an empty graph caused by a
	// dead endpoint rather than by an empty corpus.
	pub ingest_dropped_chunks: u64,
	// New remote ids refused because their phantom kern is at the entity cap.
	pub remote_cap_dropped: u64,
	// Entities dropped with no cold store bound. Spill-before-drop does not hold
	// for an in-memory kern; this is how far that deployment has diverged from a
	// durable one.
	pub unspilled_drops: u64,
	// Jobs the ingest queue refused because it was full. Nonzero means a producer
	// is outrunning the LLM leg and text was handed back, not stored.
	pub ingest_queue_refused: u64,
}

pub fn graph_health_stats(g: &GraphGnn) -> HealthStats {
	let kerns = g.all();
	let mut entities = 0usize;
	let mut reasons = 0usize;
	let mut unnamed = 0usize;
	for k in &kerns {
		entities += k.entities.len();
		reasons += k.reasons.len();
		if k.is_unnamed() {
			unnamed += 1;
		}
	}
	let gravitons: Vec<String> = crate::base::accept::root_graviton_ids(g)
		.iter()
		.filter_map(|cid| g.loaded(cid))
		.map(|c| c.graviton_text.clone())
		.collect();
	let store = g.store();
	let stamp = store
		.as_ref()
		.and_then(|s| s.embed_stamp())
		.unwrap_or_default();
	HealthStats {
		kerns: kerns.len(),
		entities,
		reasons,
		unnamed,
		gravitons,
		cold_evicted: store.as_ref().map(|s| s.cold_evicted()).unwrap_or(0),
		embed_model: stamp.model,
		// An unstamped store still knows what it indexed.
		embed_dim: match stamp.dim {
			0 => g.entity_vector_dim().unwrap_or(0),
			d => d,
		},
		embed_mismatch: store.map(|s| s.embed_mismatch()).unwrap_or(false),
		query_dim_rejected: crate::base::search::query_dim_rejected(),
		below_floor_deliveries: crate::retrieval::score::below_floor_deliveries(),
		clock_skew_skips: crate::tick::stigmergy::clock_skew_skips(),
		ingest_dropped_chunks: crate::ingest::worker::ingest_dropped_chunks(),
		remote_cap_dropped: crate::base::merge::remote_cap_dropped(),
		unspilled_drops: crate::tick::stigmergy::unspilled_drops(),
		ingest_queue_refused: crate::ingest::worker::ingest_queue_refused(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::store::{EmbedStamp, Store};

	#[test]
	fn empty_graph_reports_no_entities_or_reasons() {
		let g = GraphGnn::new();
		let h = graph_health_stats(&g);
		assert_eq!(h.entities, 0, "fresh graph has no entities");
		assert_eq!(h.reasons, 0, "fresh graph has no reasons");
		assert!(h.kerns >= 1, "at least the root kern is present");
		assert!(h.gravitons.len() <= h.kerns);
	}

	#[test]
	fn storeless_graph_reports_zeroed_store_signals() {
		let h = graph_health_stats(&GraphGnn::new());
		assert_eq!(h.cold_evicted, 0);
		assert!(h.embed_model.is_empty());
		assert_eq!(h.embed_dim, 0);
		assert!(!h.embed_mismatch);
	}

	#[test]
	fn store_signals_surface_evictions_and_the_embed_stamp() {
		use crate::base::types::{mk_entity, EntityKind};

		let d = tempfile::tempdir().unwrap();
		let store = Store::open(&d.path().to_string_lossy()).unwrap();
		for i in 0..3 {
			let mut e = mk_entity(&format!("e{i}"), "cold", 0.0, EntityKind::Claim);
			e.created_at =
				Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(100 * (i as u64 + 1)));
			store.cold_spill(&e).unwrap();
		}
		store
			.check_embed_stamp(&EmbedStamp {
				model: "qwen3".into(),
				dim: 1024,
			})
			.unwrap();
		store
			.check_embed_stamp(&EmbedStamp {
				model: "nomic".into(),
				dim: 768,
			})
			.unwrap();

		let mut g = GraphGnn::new();
		g.set_store(std::sync::Arc::new(store));
		let h = graph_health_stats(&g);
		assert_eq!(h.embed_model, "qwen3", "health names the STORED model");
		assert_eq!(h.embed_dim, 1024);
		assert!(h.embed_mismatch, "the model swap is visible to an operator");
		assert_eq!(h.cold_evicted, 0, "under the cap nothing is evicted");

		g.store().unwrap().cold_cap(1).unwrap();
		assert_eq!(
			graph_health_stats(&g).cold_evicted,
			2,
			"evicted rows are reported, not silently dropped"
		);
	}
}
