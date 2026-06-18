use crate::base::constants::INGEST_DEDUP_THRESHOLD;

/// Runtime ingest knobs threaded through the [`Worker`](crate::ingest::Worker)
/// pipeline. Distinct from the serde-deserialized
/// [`IngestConfig`](crate::config::IngestConfig): this runtime form carries
/// `ttl_secs`. Both share the same default values via the `INGEST_*`
/// constants in `base::constants`.
#[derive(Debug, Clone)]
pub struct Config {
	/// Cosine-similarity floor in `[0.0, 1.0]`: a new vector whose nearest
	/// neighbour scores at or above this is treated as a duplicate and merged
	/// instead of inserted. Higher → fewer merges (stricter "same thought").
	pub dedup_threshold: f64,
	/// Optional time-to-live, in seconds, applied to ingested entities. `None`
	/// means no expiry (the default).
	pub ttl_secs: Option<u64>,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			ttl_secs: None,
		}
	}
}

impl Config {
	/// Reject an out-of-range configuration at construction time rather than
	/// letting a bad knob surface as silently-wrong behaviour deep in ingest.
	/// The dedup similarity floor must lie in `[0.0, 1.0]`.
	pub fn validate(&self) -> Result<(), String> {
		if !(0.0..=1.0).contains(&self.dedup_threshold) {
			return Err(format!(
				"dedup_threshold must be in [0.0, 1.0], got {}",
				self.dedup_threshold
			));
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The runtime `Config` and the serde `IngestConfig` describe the same knobs
	/// at two layers; their defaults must agree. Both now source the shared
	/// `INGEST_*` constants, so this guards against a future edit re-introducing a
	/// divergent literal in one layer.
	#[test]
	fn runtime_and_serde_ingest_defaults_agree() {
		let rt = Config::default();
		let serde = crate::config::IngestConfig::default();
		assert_eq!(rt.dedup_threshold, serde.dedup_threshold);
		// And both trace back to the shared constants.
		assert_eq!(rt.dedup_threshold, INGEST_DEDUP_THRESHOLD);
	}

	#[test]
	fn validate_accepts_the_default_and_rejects_bad_knobs() {
		assert!(
			Config::default().validate().is_ok(),
			"default config is valid"
		);

		let out_of_range = Config {
			dedup_threshold: 1.5,
			..Default::default()
		};
		assert!(out_of_range
			.validate()
			.unwrap_err()
			.contains("dedup_threshold"));
	}
}
