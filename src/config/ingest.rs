use serde::{Deserialize, Serialize};

use crate::base::constants::INGEST_DEDUP_THRESHOLD;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
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
	pub fn validate(&self) -> Result<(), String> {
		crate::ingest::Config {
			dedup_threshold: self.dedup_threshold,
			valid_from: None,
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
