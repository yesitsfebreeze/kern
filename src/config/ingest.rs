use serde::{Deserialize, Serialize};

use crate::base::constants::INGEST_DEDUP_THRESHOLD;

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
}

impl Default for IngestConfig {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
		}
	}
}

impl IngestConfig {
	/// Reject an out-of-range ingest configuration (dedup threshold outside
	/// `[0,1]`). Delegates to the single canonical range check on the runtime
	/// [`ingest::Config`](crate::ingest::Config) — mapping these serde fields onto
	/// it — so the two config layers can never validate differently.
	pub fn validate(&self) -> Result<(), String> {
		crate::ingest::Config {
			dedup_threshold: self.dedup_threshold,
			ttl_secs: None,
		}
		.validate()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_validates_and_bad_knobs_are_rejected() {
		assert!(
			IngestConfig::default().validate().is_ok(),
			"shipped defaults are valid"
		);

		let out_of_range = IngestConfig {
			dedup_threshold: 2.0,
		};
		assert!(
			out_of_range.validate().is_err(),
			"threshold outside [0,1] is rejected"
		);
	}
}
