use serde::{Deserialize, Serialize};

use crate::base::constants::{TICK_INTERVAL_SECS, TICK_MAX_CLUSTER_SAMPLE, TICK_QUEUE_CAPACITY};

/// Serde-deserialized (`[tick]` in `kern.toml`) tuning for the autonomous
/// maintenance driver. Defaults come from the `TICK_*` constants in
/// `base::constants` so the baseline lives in one place.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct TickConfig {
	/// Max number of entities sampled when clustering a kern for auto-naming and
	/// child-spawn. Bounds the clustering cost on large kerns; above this size the
	/// cluster pass works on a sample rather than every entity.
	pub max_cluster_sample: usize,
	/// Bounded capacity of the maintenance-tick task queue (`Queue::new`, floored
	/// at 1). Sizes how much pending tick work can queue before backpressure.
	pub queue_capacity: usize,
	/// Seconds between autonomous maintenance ticks (heat decay + stigmergy GC via
	/// `pulse`, plus re-enqueuing clustering). `0` disables the driver, leaving
	/// compaction event-driven only. Without this an idle daemon never decays or
	/// evicts cold nodes.
	pub interval_secs: u64,
}

impl Default for TickConfig {
	fn default() -> Self {
		Self {
			max_cluster_sample: TICK_MAX_CLUSTER_SAMPLE,
			queue_capacity: TICK_QUEUE_CAPACITY,
			interval_secs: TICK_INTERVAL_SECS,
		}
	}
}
