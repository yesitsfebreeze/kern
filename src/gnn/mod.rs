//! GNN subsystem: kern's learned re-embedder.
//!
//! A small from-scratch graph neural network that periodically re-embeds the
//! entity graph so the GNN index (`gnn_entity_idx` on [`crate::base::graph`])
//! captures structural/relational signal the raw content embeddings miss. The
//! tick loop trains it on the live graph and writes back per-node `gnn_vector`s,
//! which retrieval fuses with content similarity (see `base::search::merge_hits`).
//!
//! The [`gcn`] layer implements the [`GraphLayer`] / [`BackwardGraphLayer`]
//! traits; [`loss`] and [`optim`] drive the training step (run inline by
//! [`propagate`]); [`tensor`] is the minimal dense-matrix backbone (no external
//! BLAS). Operation errors surface as [`GnnError`].

pub mod activation;
pub mod backward;

pub use activation::Activation;
pub use backward::{BackwardGraphLayer, GraphLayer};

/// Errors raised by GNN layer operations.
///
/// Reused across gnn submodules; extend this enum rather than introducing
/// per-site error types.
#[derive(Debug, thiserror::Error)]
pub enum GnnError {
	/// `backward_graph` / inference invoked before a successful `forward_graph`,
	/// or after state was reset. Cached forward state is missing.
	#[error("gnn: missing forward state ({0}); call forward_graph before backward/inference")]
	MissingForwardState(&'static str),

	/// Tensor-level shape error bubbled up from cached intermediates.
	#[error("gnn: tensor error: {0}")]
	Tensor(#[from] crate::gnn::tensor::TensorError),
}

pub mod dropout;
pub mod gcn;
pub mod graph;
pub mod layer;
pub mod loss;
pub mod model;
pub mod norm;
pub mod optim;
pub mod persist;
pub mod propagate;
pub mod tensor;
