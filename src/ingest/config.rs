use crate::base::constants::INGEST_DEDUP_THRESHOLD;
use crate::base::types::{ReviewState, Source};

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// Source scheme → the curation state a claim arriving on it is placed in. An
/// absent key is `Active`, so an empty policy is today's behaviour exactly.
pub type ReviewPolicy = BTreeMap<String, ReviewState>;

/// The one resolution, so no producer can key on something other than the
/// scheme `IngestConfig::validate` checks against.
pub fn review_for(policy: &ReviewPolicy, source: &Source) -> ReviewState {
	policy.get(source.scheme()).copied().unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Config {
	pub dedup_threshold: f64,
	pub valid_from: Option<std::time::SystemTime>,
	pub valid_until: Option<std::time::SystemTime>,
	// The POLICY, not a resolved state: the intake drain hands one `Config` to a
	// whole pass of records whose sources differ, so the scheme is only known
	// per job. `job()` resolves it — the single gate every producer passes.
	pub review_policy: ReviewPolicy,
}

impl Default for Config {
	fn default() -> Self {
		Self {
			dedup_threshold: INGEST_DEDUP_THRESHOLD,
			valid_from: None,
			valid_until: None,
			review_policy: ReviewPolicy::new(),
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
	/// The same conversion, resolved *now*, for the entrances whose retention is
	/// a standing policy rather than a per-call argument. Long-lived callers (the
	/// intake poll loop, the file watcher) must build one per pass: resolving it
	/// once at startup would stamp a file seen on day 30 with a deadline measured
	/// from boot. `Config::validate` refuses an unrepresentable retention at
	/// load, so an error here is a caller that skipped it — say so, then no TTL.
	pub fn with_retention(mut self, retention_secs: u64) -> Self {
		self.valid_until = valid_until_from_retention(retention_secs).unwrap_or_else(|e| {
			tracing::error!(target: "kern.ingest", error = %e, "unusable retention_secs; ingesting with no TTL");
			None
		});
		self
	}

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
		assert_eq!(rt.review_policy, serde.review_policy);
		assert!(
			rt.review_policy.is_empty(),
			"nothing is held for review until a host asks"
		);
	}

	#[test]
	fn review_for_keys_on_the_scheme_and_defaults_to_active() {
		let file = Source::File {
			path: "/a".into(),
			section: String::new(),
			title: String::new(),
			author: String::new(),
			url: String::new(),
		};
		let inline = Source::Inline {
			hash: "h".into(),
			section: String::new(),
		};
		let policy = ReviewPolicy::from([("file".to_string(), ReviewState::Pending)]);
		assert_eq!(review_for(&policy, &file), ReviewState::Pending);
		assert_eq!(
			review_for(&policy, &inline),
			ReviewState::Active,
			"an unlisted scheme is active — the policy holds back only what it names"
		);
		assert_eq!(
			review_for(&ReviewPolicy::new(), &file),
			ReviewState::Active,
			"an empty policy holds nothing back"
		);
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
	fn with_retention_carries_a_standing_policy_onto_the_config() {
		let before = SystemTime::now();
		let cfg = Config {
			dedup_threshold: 0.9,
			..Default::default()
		}
		.with_retention(3600);
		let got = cfg
			.valid_until
			.expect("a policy retention yields a deadline");
		assert!(
			got >= before + Duration::from_secs(3600),
			"the deadline is resolved at call time, not at startup"
		);
		assert_eq!(cfg.dedup_threshold, 0.9, "the other knobs survive");

		assert_eq!(
			Config::default().with_retention(0).valid_until,
			None,
			"no configured policy means no TTL"
		);
	}

	#[test]
	fn retention_that_overflows_the_clock_is_rejected_loudly() {
		assert!(valid_until_from_retention(u64::MAX)
			.unwrap_err()
			.contains("overflows the clock"));
	}
}
