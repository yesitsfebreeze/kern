use crate::base::constants::INGEST_DEDUP_THRESHOLD;

/// Runtime ingest knobs. Distinct from the serde
/// [`IngestConfig`](crate::config::IngestConfig): this form carries `ttl_secs`.
#[derive(Debug, Clone)]
pub struct Config {
	/// Cosine-similarity floor in `[0.0, 1.0]`; at or above it a new vector
	/// merges into its nearest neighbour instead of inserting.
	pub dedup_threshold: f64,
	/// Seconds; `None` = no expiry.
	pub ttl_secs: Option<u64>,
	/// World-time start from a distilled `valid_from` hint; `None` falls back to
	/// the ingestion time.
	pub valid_from: Option<std::time::SystemTime>,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			ttl_secs: None,
			valid_from: None,
		}
	}
}

impl Config {
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

	#[test]
	fn runtime_and_serde_ingest_defaults_agree() {
		let rt = Config::default();
		let serde = crate::config::IngestConfig::default();
		assert_eq!(rt.dedup_threshold, serde.dedup_threshold);
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
