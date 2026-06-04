use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct TickConfig {
	pub unnamed_stall_threshold: usize,
	pub max_cluster_sample: usize,
	pub queue_capacity: usize,
	/// Seconds between autonomous maintenance ticks (heat decay + stigmergy
	/// GC via `pulse`, plus re-enqueuing clustering). `0` disables the
	/// driver, leaving compaction event-driven only. Without this an idle
	/// daemon never decays or evicts cold nodes.
	pub interval_secs: u64,
}

impl Default for TickConfig {
	fn default() -> Self {
		Self {
			unnamed_stall_threshold: 10,
			max_cluster_sample: 200,
			queue_capacity: 512,
			interval_secs: 60,
		}
	}
}
