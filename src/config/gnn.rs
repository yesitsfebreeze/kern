use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnConfig {
	pub self_weight: f64,
	pub min_weight: f64,
	pub min_thoughts: usize,
	pub train_epochs: usize,
	pub train_learning_rate: f64,
}

impl Default for GnnConfig {
	fn default() -> Self {
		Self {
			// Defaults live once in gnn::propagate (shared with the runtime
			// GnnConfig) so the serde and runtime layers cannot drift.
			self_weight: crate::gnn::propagate::DEFAULT_SELF_WEIGHT,
			min_weight: crate::gnn::propagate::DEFAULT_MIN_WEIGHT,
			min_thoughts: crate::gnn::propagate::DEFAULT_MIN_THOUGHTS,
			train_epochs: crate::gnn::propagate::DEFAULT_TRAIN_EPOCHS,
			train_learning_rate: crate::gnn::propagate::DEFAULT_TRAIN_LEARNING_RATE,
		}
	}
}

impl From<GnnConfig> for crate::gnn::propagate::GnnConfig {
	fn from(c: GnnConfig) -> Self {
		crate::gnn::propagate::GnnConfig {
			self_weight: c.self_weight,
			min_weight: c.min_weight,
			min_thoughts: c.min_thoughts,
			train_epochs: c.train_epochs,
			train_learning_rate: c.train_learning_rate,
		}
	}
}
