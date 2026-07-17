use serde::{Deserialize, Serialize};

use crate::base::constants::KERN_CAP_DISABLED;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
	pub max_kerns: usize,
	pub max_ledger_entries: usize,
	/// Entity count above which `rebuild_index` spills the vector index to a
	/// disk-resident DiskANN snapshot. [`KERN_CAP_DISABLED`] = never spill.
	pub disk_threshold: usize,
}

impl Default for GraphConfig {
	fn default() -> Self {
		Self {
			// Do NOT set a finite cap: eviction drops unpersisted `children` pushes,
			// re-spawning a child every tick until the graph fragments. Fix the
			// evict/persist bug first.
			max_kerns: KERN_CAP_DISABLED,
			max_ledger_entries: 10_000,
			disk_threshold: KERN_CAP_DISABLED,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_disables_kern_eviction() {
		// Do NOT relax: a finite cap corrupts the graph (see GraphConfig::default).
		assert_eq!(GraphConfig::default().max_kerns, KERN_CAP_DISABLED);
		assert_eq!(
			KERN_CAP_DISABLED,
			usize::MAX,
			"sentinel value is the uncapped marker"
		);
	}

	#[test]
	fn default_disables_disk_spill() {
		assert_eq!(GraphConfig::default().disk_threshold, KERN_CAP_DISABLED);
	}
}
