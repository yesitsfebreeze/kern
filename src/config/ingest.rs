use serde::{Deserialize, Serialize};

use crate::base::constants::{
	INGEST_DEDUP_THRESHOLD, INGEST_HNSW_EF, INGEST_HNSW_K, INGEST_REPHRASE_LOWER,
	INGEST_REPHRASE_UPPER,
};

/// Serde-deserialized (`kern.toml`) ingest tuning. Mirrors the runtime
/// [`ingest::Config`](crate::ingest::Config) knobs (which additionally carries
/// `ttl_secs`); both default to the shared `INGEST_*` constants in
/// `base::constants` so the two layers cannot drift.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
	/// Cosine-similarity floor in `[0.0, 1.0]`: a new vector whose nearest
	/// neighbour scores at or above this is merged as a duplicate of that entity
	/// instead of inserted. Higher → stricter "same thought" (fewer merges).
	pub dedup_threshold: f64,
	/// Nearest-neighbour count (`k`) for the synthesis/rephrase HNSW probe — how
	/// many existing entities are weighed as rephrase candidates per ingest.
	pub hnsw_k: usize,
	/// HNSW search beam width (`ef`) for that probe. Wider → better recall of the
	/// true nearest neighbours at higher search cost. Must be `>= hnsw_k`.
	pub hnsw_ef: usize,
	/// Lower edge of the rephrase similarity band in `[0.0, 1.0]`: a candidate at
	/// or below this is too dissimilar to merge and stays a separate entity.
	pub rephrase_lower: f64,
	/// Upper edge of the rephrase band: a candidate at or above this is a
	/// near-duplicate (handled by dedup). Only candidates STRICTLY between
	/// `rephrase_lower` and `rephrase_upper` are rephrased/merged.
	pub rephrase_upper: f64,
	/// Max number of `fork_id`s remembered in the session-mirror dedup set
	/// before FIFO eviction kicks in. Bounds memory under long-running
	/// daemons that accumulate many forks.
	pub session_mirror_max_seen: usize,
}

impl Default for IngestConfig {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			hnsw_k: INGEST_HNSW_K,
			hnsw_ef: INGEST_HNSW_EF,
			rephrase_lower: INGEST_REPHRASE_LOWER,
			rephrase_upper: INGEST_REPHRASE_UPPER,
			session_mirror_max_seen: 4096,
		}
	}
}

impl IngestConfig {
	/// Reject an out-of-range ingest configuration (thresholds outside `[0,1]`, an
	/// inverted rephrase band, `hnsw_k == 0`, or `hnsw_ef < hnsw_k`). Delegates to
	/// the single canonical range check on the runtime
	/// [`ingest::Config`](crate::ingest::Config) — mapping these serde fields onto
	/// it — so the two config layers can never validate differently.
	pub fn validate(&self) -> Result<(), String> {
		crate::ingest::Config {
			dedup_threshold: self.dedup_threshold,
			ttl_secs: None,
			hnsw_k: self.hnsw_k,
			hnsw_ef: self.hnsw_ef,
			rephrase_lower: self.rephrase_lower,
			rephrase_upper: self.rephrase_upper,
		}
		.validate()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_validates_and_bad_knobs_are_rejected() {
		assert!(IngestConfig::default().validate().is_ok(), "shipped defaults are valid");

		let inverted = IngestConfig { rephrase_lower: 0.9, rephrase_upper: 0.8, ..Default::default() };
		assert!(inverted.validate().is_err(), "inverted rephrase band is rejected");

		let out_of_range = IngestConfig { dedup_threshold: 2.0, ..Default::default() };
		assert!(out_of_range.validate().is_err(), "threshold outside [0,1] is rejected");

		let narrow_beam = IngestConfig { hnsw_k: 16, hnsw_ef: 8, ..Default::default() };
		assert!(narrow_beam.validate().is_err(), "ef < k is rejected");
	}
}
