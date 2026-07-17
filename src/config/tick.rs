use serde::{Deserialize, Serialize};

use crate::base::constants::{TICK_INTERVAL_SECS, TICK_MAX_CLUSTER_SAMPLE, TICK_QUEUE_CAPACITY};

/// `[tick]`: the autonomous maintenance driver. Defaults come from the `TICK_*`
/// constants in `base::constants`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct TickConfig {
	/// Entities sampled when clustering a kern; above this the pass works on a
	/// sample, not every entity.
	pub max_cluster_sample: usize,
	/// Task-queue capacity (`Queue::new`, floored at 1) before backpressure.
	pub queue_capacity: usize,
	/// Seconds between maintenance ticks. `0` disables the driver — an idle
	/// daemon then never decays heat or evicts cold nodes.
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
