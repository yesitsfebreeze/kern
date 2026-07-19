use serde::{Deserialize, Serialize};

// `dir` and `digest_path` MUST stay cwd-relative and independent of `data_dir`:
// the MCP server resolves them from session cwd; deriving from data_dir breaks that contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
	pub enabled: bool,
	pub dir: String,
	pub poll_secs: u64,
	pub digest_path: String,
	pub digest_secs: u64,
	pub digest_k: usize,
	pub digest_min_trust: f64,
	pub digest_token_budget: usize,
	pub done_retention_secs: u64,
}

impl Default for CaptureConfig {
	fn default() -> Self {
		Self {
			enabled: true,
			dir: ".kern/capture".into(),
			poll_secs: 5,
			digest_path: ".kern/digest.md".into(),
			digest_secs: 30,
			digest_k: 40,
			digest_min_trust: 0.35,
			digest_token_budget: 1500,
			done_retention_secs: 7 * 24 * 60 * 60,
		}
	}
}

impl CaptureConfig {
	pub fn validate(&self) -> Result<(), String> {
		if self.enabled {
			if self.poll_secs == 0 {
				return Err("poll_secs must be > 0 (0 busy-loops the intake drain)".into());
			}
			if self.digest_secs == 0 {
				return Err("digest_secs must be > 0 (0 busy-loops the digest rebuild)".into());
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_are_on_with_sane_tunables() {
		let c = CaptureConfig::default();
		assert!(c.enabled);
		assert_eq!(c.dir, ".kern/capture");
		assert_eq!(c.poll_secs, 5);
		assert_eq!(c.digest_path, ".kern/digest.md");
		assert_eq!(c.digest_secs, 30);
		assert_eq!(c.digest_k, 40);
		assert_eq!(c.digest_min_trust, 0.35);
		assert_eq!(c.digest_token_budget, 1500);
		assert_eq!(c.done_retention_secs, 604_800, "7 days in seconds");
	}

	#[test]
	fn validate_rejects_zero_intervals_only_when_enabled() {
		assert!(
			CaptureConfig::default().validate().is_ok(),
			"default (enabled, non-zero intervals) is valid"
		);

		let enabled = CaptureConfig {
			enabled: true,
			..Default::default()
		};
		assert!(
			enabled.validate().is_ok(),
			"enabled default intervals are non-zero"
		);

		let zero_poll = CaptureConfig {
			enabled: true,
			poll_secs: 0,
			..Default::default()
		};
		assert!(zero_poll.validate().unwrap_err().contains("poll_secs"));

		let zero_digest = CaptureConfig {
			enabled: true,
			digest_secs: 0,
			..Default::default()
		};
		assert!(zero_digest.validate().unwrap_err().contains("digest_secs"));

		let disabled_zero = CaptureConfig {
			enabled: false,
			poll_secs: 0,
			digest_secs: 0,
			..Default::default()
		};
		assert!(
			disabled_zero.validate().is_ok(),
			"disabled capture ignores its intervals"
		);
	}
}
