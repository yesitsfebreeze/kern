use serde::{Deserialize, Serialize};

/// Configuration for Claude-Code memory capture + recall.
///
/// OFF by default. Opt in via a `[capture]` section in `.relay/kern.toml`:
///
/// ```toml
/// [capture]
/// enabled = true
/// ```
///
/// `dir` and `digest_path` are intentionally **cwd-relative and independent
/// of `data_dir`**: the Claude-Code hooks (`kern-capture.mjs`,
/// `kern-recall.mjs`) resolve these paths from the session cwd and have no
/// knowledge of kern's `data_dir`. Do not derive them from `data_dir` — that
/// would break the hook contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
	/// Master switch for the capture_spool + digest tasks.
	pub enabled: bool,
	/// Spool directory (relative to cwd) the Stop hook writes deltas into.
	pub dir: String,
	/// How often the spool is drained, in seconds.
	pub poll_secs: u64,
	/// Output path (relative to cwd) for the recall digest.
	pub digest_path: String,
	/// How often the digest is regenerated, in seconds.
	pub digest_secs: u64,
	/// Max thoughts included in the digest.
	pub digest_k: usize,
}

impl Default for CaptureConfig {
	fn default() -> Self {
		Self {
			enabled: false,
			dir: ".relay/capture".into(),
			poll_secs: 5,
			digest_path: ".relay/kern/digest.md".into(),
			digest_secs: 30,
			digest_k: 40,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_are_off_with_sane_tunables() {
		let c = CaptureConfig::default();
		assert!(!c.enabled);
		assert_eq!(c.dir, ".relay/capture");
		assert_eq!(c.poll_secs, 5);
		assert_eq!(c.digest_path, ".relay/kern/digest.md");
		assert_eq!(c.digest_secs, 30);
		assert_eq!(c.digest_k, 40);
	}
}
