use serde::{Deserialize, Serialize};

// `dir` MUST stay cwd-relative and independent of `data_dir`:
// the MCP server resolves it from session cwd; deriving from data_dir breaks that contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntakeConfig {
	pub enabled: bool,
	pub dir: String,
	pub poll_secs: u64,
	pub done_retention_secs: u64,
}

impl Default for IntakeConfig {
	fn default() -> Self {
		Self {
			enabled: true,
			dir: ".kern/intake".into(),
			poll_secs: 5,
			done_retention_secs: 7 * 24 * 60 * 60,
		}
	}
}

impl IntakeConfig {
	pub fn validate(&self) -> Result<(), String> {
		if self.enabled && self.poll_secs == 0 {
			return Err("poll_secs must be > 0 (0 busy-loops the intake drain)".into());
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_are_on_with_sane_tunables() {
		let c = IntakeConfig::default();
		assert!(c.enabled);
		assert_eq!(c.dir, ".kern/intake");
		assert_eq!(c.poll_secs, 5);
		assert_eq!(c.done_retention_secs, 604_800, "7 days in seconds");
	}

	#[test]
	fn validate_rejects_zero_poll_only_when_enabled() {
		assert!(
			IntakeConfig::default().validate().is_ok(),
			"default (enabled, non-zero poll) is valid"
		);

		let zero_poll = IntakeConfig {
			enabled: true,
			poll_secs: 0,
			..Default::default()
		};
		assert!(zero_poll.validate().unwrap_err().contains("poll_secs"));

		let disabled_zero = IntakeConfig {
			enabled: false,
			poll_secs: 0,
			..Default::default()
		};
		assert!(
			disabled_zero.validate().is_ok(),
			"disabled intake ignores its poll interval"
		);
	}
}
