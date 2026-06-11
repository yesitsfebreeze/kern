use serde::{Deserialize, Serialize};

use crate::base::constants::KERN_CAP_DISABLED;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
	pub max_kerns: usize,
	pub max_ledger_entries: usize,
}

impl Default for GraphConfig {
	fn default() -> Self {
		Self {
			// Kern eviction is DISABLED by default (`KERN_CAP_DISABLED` is the
			// no-cap sentinel honored by GraphGnn::enforce_kern_cap). A finite cap
			// currently corrupts the graph: evicting a parent kern can drop an
			// in-memory `children` push before it is persisted, so the unnamed-
			// child lookup re-spawns a fresh child every tick — a runaway that
			// fragments the graph to `max_kerns` near-empty kerns (observed:
			// 1024 kerns / 13 entities on a real graph). Re-enable a finite cap
			// only once the evict/persist consistency bug is fixed; bounded RAM
			// for huge corpora is the DiskANN index's job, not this cap.
			// Bug tracked in kern memory — query "finite max_kerns cap evict/persist bug".
			max_kerns: KERN_CAP_DISABLED,
			max_ledger_entries: 10_000,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_disables_kern_eviction() {
		// Guards the sentinel: a future refactor must not silently re-enable a
		// finite cap while the evict/persist consistency bug is unfixed.
		assert_eq!(GraphConfig::default().max_kerns, KERN_CAP_DISABLED);
		assert_eq!(KERN_CAP_DISABLED, usize::MAX, "sentinel value is the uncapped marker");
	}
}
