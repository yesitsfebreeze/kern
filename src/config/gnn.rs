//! Serde view of the GNN re-embedder's tuning knobs (`[gnn]` in `kern.toml`).
//!
//! A thin bridge: this struct exists only so the config can be (de)serialized
//! from TOML, after which `From<GnnConfig>` converts it into the runtime
//! [`gnn::propagate::GnnConfig`](crate::gnn::propagate::GnnConfig) the re-embedder
//! actually uses. The two are field-identical; keeping them separate stops the
//! serde derives leaking into the hot runtime type, and BOTH draw their defaults
//! from the same `DEFAULT_*` consts in `gnn::propagate`, so the serde and runtime
//! layers cannot drift.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnConfig {
	/// Residual self-weight in `[0.0, 1.0]`. Each propagation step blends
	/// `self_weight * own_features + (1 - self_weight) * neighbour_message`, so a
	/// higher value keeps more of an entity's own signal (less neighbour
	/// smoothing); lower mixes in more of the graph context.
	pub self_weight: f64,
	/// Edge-weight floor: propagation ignores neighbour edges weaker than this, so
	/// near-zero links don't dilute the aggregated message.
	pub min_weight: f64,
	/// Minimum entity count before GNN training runs at all. Below it a
	/// multi-layer GNN over a tiny graph overfits, so retrieval falls back to the
	/// vector + BM25 + PageRank + reason-edge path instead.
	pub min_thoughts: usize,
	/// Number of Adam training epochs per re-embed pass.
	pub train_epochs: usize,
	/// Adam learning rate for the re-embedder's training loop.
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn from_maps_every_field_without_drift() {
		// Distinct values per field so a swapped/dropped mapping in `From` is caught.
		let serde_cfg = GnnConfig {
			self_weight: 0.11,
			min_weight: 0.22,
			min_thoughts: 33,
			train_epochs: 44,
			train_learning_rate: 0.55,
		};
		let runtime: crate::gnn::propagate::GnnConfig = serde_cfg.into();
		assert_eq!(runtime.self_weight, 0.11);
		assert_eq!(runtime.min_weight, 0.22);
		assert_eq!(runtime.min_thoughts, 33);
		assert_eq!(runtime.train_epochs, 44);
		assert_eq!(runtime.train_learning_rate, 0.55);
	}

	#[test]
	fn serde_default_equals_the_runtime_default() {
		// Both layers source the same DEFAULT_* consts; guard against divergence.
		let runtime: crate::gnn::propagate::GnnConfig = GnnConfig::default().into();
		let rd = crate::gnn::propagate::GnnConfig::defaults();
		assert_eq!(runtime.self_weight, rd.self_weight);
		assert_eq!(runtime.min_weight, rd.min_weight);
		assert_eq!(runtime.min_thoughts, rd.min_thoughts);
		assert_eq!(runtime.train_epochs, rd.train_epochs);
		assert_eq!(runtime.train_learning_rate, rd.train_learning_rate);
	}
}
