use crate::gnn::activation::Activation;
use crate::gnn::backward::{act_deriv_mul, BackwardGraphLayer, GraphLayer};
use crate::gnn::dropout::Dropout;
use crate::gnn::graph::Graph;
use crate::gnn::layer::{Backward, Layer, LinearLayer};
use crate::gnn::norm::LayerNorm;
use crate::gnn::tensor::Tensor;
use crate::gnn::GnnError;

pub struct GCNLayer {
	pub linear: LinearLayer,
	pub norm: Option<LayerNorm>,
	pub drop: Option<Dropout>,
	pub act: Option<Activation>,
	last_norm_adj: Option<Tensor>,
	last_pre_act: Option<Tensor>,
}

impl GCNLayer {
	pub fn new(
		in_features: usize,
		out_features: usize,
		act: Option<Activation>,
		norm: bool,
		drop_rate: f64,
	) -> Self {
		let mut rng = rand::rng();
		Self::with_rng(in_features, out_features, act, norm, drop_rate, &mut rng)
	}

	/// Deterministic weight init from a seeded RNG — use in tests asserting on
	/// training dynamics so the run does not depend on system entropy.
	pub fn with_rng<R: rand::Rng>(
		in_features: usize,
		out_features: usize,
		act: Option<Activation>,
		norm: bool,
		drop_rate: f64,
		rng: &mut R,
	) -> Self {
		Self {
			linear: LinearLayer::with_rng(in_features, out_features, rng),
			norm: if norm {
				Some(LayerNorm::new(out_features))
			} else {
				None
			},
			drop: if drop_rate > 0.0 {
				Some(Dropout::new(drop_rate))
			} else {
				None
			},
			act,
			last_norm_adj: None,
			last_pre_act: None,
		}
	}

	/// Fallible backward — errors instead of panicking when `forward_graph` has
	/// not run. The infallible [`BackwardGraphLayer::backward_graph`] delegates here.
	pub fn try_backward_graph(&mut self, _g: &Graph, d_out: &Tensor) -> Result<Tensor, GnnError> {
		let mut grad = d_out.clone();
		if let Some(ref d) = self.drop {
			grad = d.backward(&grad);
		}
		if let Some(a) = self.act {
			let pre_act = self
				.last_pre_act
				.as_ref()
				.ok_or(GnnError::MissingForwardState("gcn::last_pre_act"))?;
			grad = act_deriv_mul(a, &grad, pre_act);
		}
		if let Some(ref mut n) = self.norm {
			grad = n.backward(&grad);
		}
		let d_agg = self.linear.backward(&grad);
		let norm_adj = self
			.last_norm_adj
			.as_ref()
			.ok_or(GnnError::MissingForwardState("gcn::last_norm_adj"))?;
		Ok(norm_adj.transpose().matmul(&d_agg)?)
	}
}

impl GraphLayer for GCNLayer {
	fn forward_graph(&mut self, g: &Graph, features: &Tensor) -> Tensor {
		let norm_adj = g.normalized_adjacency();
		let agg = norm_adj.matmul(features).expect("GCN adj*features");
		self.last_norm_adj = Some(norm_adj);

		let mut h = self.linear.forward(&agg);
		if let Some(ref mut n) = self.norm {
			h = n.forward(&h);
		}
		self.last_pre_act = Some(h.clone());
		if let Some(a) = self.act {
			h = h.apply(|x| a.forward(x));
		}
		if let Some(ref mut d) = self.drop {
			h = d.forward(&h);
		}
		h
	}

	fn parameters(&self) -> Vec<&Tensor> {
		let mut p = self.linear.parameters();
		if let Some(ref n) = self.norm {
			p.extend(Layer::parameters(n));
		}
		p
	}

	fn parameters_mut(&mut self) -> Vec<&mut Tensor> {
		let mut p = self.linear.parameters_mut();
		if let Some(ref mut n) = self.norm {
			p.extend(Layer::parameters_mut(n));
		}
		p
	}

	fn dropout_mut(&mut self) -> Option<&mut Dropout> {
		self.drop.as_mut()
	}
}

impl BackwardGraphLayer for GCNLayer {
	fn backward_graph(&mut self, g: &Graph, d_out: &Tensor) -> Tensor {
		match self.try_backward_graph(g, d_out) {
			Ok(t) => t,
			Err(e) => {
				tracing::error!(error = %e, "GCNLayer backward_graph failed; returning zero gradient");
				// dInput is (num_nodes, in_features); in_features == linear.weight.rows.
				Tensor::zeros(g.num_nodes(), self.linear.weight.rows)
			}
		}
	}

	fn param_grads(&self) -> Vec<&Tensor> {
		let mut g = self.linear.param_grads();
		if let Some(ref n) = self.norm {
			g.extend(Backward::param_grads(n));
		}
		g
	}

	fn param_grads_mut(&mut self) -> Vec<&mut Tensor> {
		let mut g = self.linear.param_grads_mut();
		if let Some(ref mut n) = self.norm {
			g.extend(Backward::param_grads_mut(n));
		}
		g
	}

	fn zero_grads(&mut self) {
		self.linear.zero_grads();
		if let Some(ref mut n) = self.norm {
			Backward::zero_grads(n);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rand::rngs::StdRng;
	use rand::SeedableRng;

	fn two_node_graph() -> Graph {
		let mut g = Graph::new();
		g.add_node("a", vec![1.0, 0.0]).unwrap();
		g.add_node("b", vec![0.0, 1.0]).unwrap();
		g.add_edge("a", "b", vec![]).unwrap();
		g
	}

	#[test]
	fn forward_graph_aggregates_then_projects_to_out_features() {
		let g = two_node_graph();
		let feats = Tensor::new(2, 2, vec![1.0, 0.0, 0.0, 1.0]).unwrap();
		let mut rng = StdRng::seed_from_u64(1);
		let mut layer = GCNLayer::with_rng(2, 3, None, false, 0.0, &mut rng);

		let out = layer.forward_graph(&g, &feats);
		assert_eq!((out.rows, out.cols), (2, 3), "num_nodes x out_features");
		let adj = layer
			.last_norm_adj
			.as_ref()
			.expect("normalized adjacency cached");
		assert_eq!(
			(adj.rows, adj.cols),
			(2, 2),
			"adjacency is num_nodes x num_nodes"
		);
		assert!(
			layer.last_pre_act.is_some(),
			"pre-activation cached for backward"
		);
	}

	#[test]
	fn try_backward_before_forward_is_missing_state_and_infallible_path_zeroes() {
		let g = two_node_graph();
		let mut rng = StdRng::seed_from_u64(2);
		// With an activation, the act path's last_pre_act guard trips first.
		let mut layer = GCNLayer::with_rng(2, 3, Some(Activation::Relu), false, 0.0, &mut rng);
		let d_out = Tensor::ones(2, 3);

		assert!(matches!(
			layer.try_backward_graph(&g, &d_out).unwrap_err(),
			GnnError::MissingForwardState(_)
		));
		let z = layer.backward_graph(&g, &d_out);
		assert_eq!(
			(z.rows, z.cols),
			(2, 2),
			"fallback dInput is num_nodes x in_features"
		);
		assert!(z.data.iter().all(|&v| v == 0.0));
	}
}
