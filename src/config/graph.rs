use serde::{Deserialize, Serialize};

use crate::base::constants::KERN_CAP_DISABLED;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
	pub max_kerns: usize,
	pub max_ledger_entries: usize,
	/// Resident searchable-entity count above which `rebuild_index` spills the
	/// entity vector index to a disk-resident DiskANN (Vamana) snapshot instead of
	/// holding every vector in the in-RAM HNSW — the bounded-RAM path for huge
	/// corpora the `max_kerns` comment defers to. [`KERN_CAP_DISABLED`] (the
	/// default) means "never spill": small deployments keep the in-RAM index and
	/// behave exactly as before.
	pub disk_threshold: usize,
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
			// Disk spill OFF by default — never crosses any real entity count, so
			// the in-RAM HNSW stays the index until an operator opts in.
			disk_threshold: KERN_CAP_DISABLED,
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

	#[test]
	fn default_disables_disk_spill() {
		// The disk-spill threshold ships OFF so small deployments keep the in-RAM
		// index and behave exactly as before; an operator opts in explicitly.
		assert_eq!(GraphConfig::default().disk_threshold, KERN_CAP_DISABLED);
	}
}
