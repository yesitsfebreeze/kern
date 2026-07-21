use crate::base::constants::INGEST_DEDUP_THRESHOLD;

use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct Config {
	pub dedup_threshold: f64,
	pub valid_from: Option<std::time::SystemTime>,
	pub valid_until: Option<std::time::SystemTime>,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			valid_from: None,
			valid_until: None,
		}
	}
}

// Retention is a duration at the caller boundary and an absolute instant on the
// entity. The single conversion lives here so the CLI flag and the MCP field
// cannot drift apart; 0 means "no TTL", matching every other unset knob.
pub fn valid_until_from_retention(retention_secs: u64) -> Result<Option<SystemTime>, String> {
	if retention_secs == 0 {
		return Ok(None);
	}
	SystemTime::now()
		.checked_add(Duration::from_secs(retention_secs))
		.map(Some)
		.ok_or_else(|| format!("retention_secs {retention_secs} overflows the clock"))
}

impl Config {
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

	#[test]
	fn retention_becomes_an_absolute_deadline_one_hour_out() {
		let before = SystemTime::now();
		let got = valid_until_from_retention(3600)
			.expect("an hour is representable")
			.expect("a non-zero retention yields a deadline");
		let after = SystemTime::now();
		assert!(
			got >= before + Duration::from_secs(3600) && got <= after + Duration::from_secs(3600),
			"valid_until is now + 1h"
		);
	}

	#[test]
	fn omitted_retention_leaves_no_deadline() {
		assert_eq!(
			valid_until_from_retention(0).expect("zero is not an error"),
			None,
			"0 means no TTL"
		);
		assert_eq!(
			Config::default().valid_until,
			None,
			"a default ingest sets no valid_until"
		);
	}

	#[test]
	fn retention_that_overflows_the_clock_is_rejected_loudly() {
		assert!(valid_until_from_retention(u64::MAX)
			.unwrap_err()
			.contains("overflows the clock"));
	}
}
