pub mod activation;
pub mod backward;

pub use activation::Activation;
pub use backward::{BackwardGraphLayer, GraphLayer};

#[derive(Debug, thiserror::Error)]
pub enum GnnError {
	#[error("gnn: missing forward state ({0}); call forward_graph before backward/inference")]
	MissingForwardState(&'static str),

	#[error("gnn: tensor error: {0}")]
	Tensor(#[from] crate::gnn::tensor::TensorError),
}

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
