use serde::{Deserialize, Serialize};

/// Default days of `history.db` rows retained before startup pruning.
pub const DEFAULT_RETAIN_DAYS: u32 = 30;
/// Default soft cap on `today.jsonl` (bytes) before a forced mid-day rollover.
/// 50 MiB. NB the `journal` crate keeps its own private 50-MiB standalone default
/// (`day_journal::DEFAULT_MAX_TODAY_BYTES`) for when kern does not override it;
/// the two encode the same number independently across the crate boundary.
pub const DEFAULT_MAX_TODAY_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct JournalConfig {
	/// Number of days of history.db rows to keep. Older rows are pruned
	/// at kern startup. `0` disables pruning.
	pub retain_days: u32,
	/// Soft cap on today.jsonl size in bytes before forcing a mid-day
	/// rollover (current contents flushed into history.db, file rewritten).
	/// `0` disables the cap (size-based rollover is skipped; only a day change
	/// rolls over). Default 50 MiB.
	pub max_today_bytes: u64,
}

impl Default for JournalConfig {
	fn default() -> Self {
		Self {
			retain_days: DEFAULT_RETAIN_DAYS,
			max_today_bytes: DEFAULT_MAX_TODAY_BYTES,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_matches_the_exposed_constants() {
		let d = JournalConfig::default();
		assert_eq!(d.retain_days, DEFAULT_RETAIN_DAYS);
		assert_eq!(d.max_today_bytes, DEFAULT_MAX_TODAY_BYTES);
		assert_eq!(DEFAULT_MAX_TODAY_BYTES, 50 * 1024 * 1024, "50 MiB");
	}
}
