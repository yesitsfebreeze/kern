use crate::gnn::backward::{BackwardGraphLayer, GraphLayer};
use crate::gnn::graph::Graph;
use crate::gnn::layer::{Backward, Layer, LinearLayer};
use crate::gnn::tensor::Tensor;

// A `forward` must precede its `backward`; call `zero_grads` before each backward.
pub struct Model {
	pub layers: Vec<Box<dyn BackwardGraphLayer>>,
	pub out_layer: Option<LinearLayer>,
}

impl Model {
	pub fn new(layers: Vec<Box<dyn BackwardGraphLayer>>, out_layer: Option<LinearLayer>) -> Self {
		Self { layers, out_layer }
	}

	pub fn forward(&mut self, g: &Graph, features: &Tensor) -> Tensor {
		let mut h = features.clone();
		for layer in &mut self.layers {
			h = layer.forward_graph(g, &h);
		}
		if let Some(ref mut ol) = self.out_layer {
			h = ol.forward(&h);
		}
		h
	}

	pub fn backward(&mut self, g: &Graph, d_out: &Tensor) {
		let mut grad = d_out.clone();
		if let Some(ref mut ol) = self.out_layer {
			grad = ol.backward(&grad);
		}
		for layer in self.layers.iter_mut().rev() {
			grad = layer.backward_graph(g, &grad);
		}
	}

	pub fn parameters(&self) -> Vec<&Tensor> {
		let mut p = Vec::new();
		for layer in &self.layers {
			p.extend(GraphLayer::parameters(layer.as_ref()));
		}
		if let Some(ref ol) = self.out_layer {
			p.extend(Layer::parameters(ol));
		}
		p
	}

	pub fn parameters_mut(&mut self) -> Vec<&mut Tensor> {
		let mut p = Vec::new();
		for layer in &mut self.layers {
			p.extend(GraphLayer::parameters_mut(layer.as_mut()));
		}
		if let Some(ref mut ol) = self.out_layer {
			p.extend(Layer::parameters_mut(ol));
		}
		p
	}

	pub fn param_grads(&self) -> Vec<&Tensor> {
		let mut g = Vec::new();
		for layer in &self.layers {
			g.extend(layer.param_grads());
		}
		if let Some(ref ol) = self.out_layer {
			g.extend(Backward::param_grads(ol));
		}
		g
	}

	pub fn param_grads_mut(&mut self) -> Vec<&mut Tensor> {
		let mut g = Vec::new();
		for layer in &mut self.layers {
			g.extend(layer.param_grads_mut());
		}
		if let Some(ref mut ol) = self.out_layer {
			g.extend(Backward::param_grads_mut(ol));
		}
		g
	}

	pub fn zero_grads(&mut self) {
		for layer in &mut self.layers {
			layer.zero_grads();
		}
		if let Some(ref mut ol) = self.out_layer {
			Backward::zero_grads(ol);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gnn::gcn::GCNLayer;
	use rand::rngs::StdRng;
	use rand::SeedableRng;

	fn tiny_graph() -> (Graph, Tensor) {
		let mut g = Graph::new();
		let feats = [
			[0.5, -0.2, 0.1, 0.3],
			[-0.4, 0.6, 0.2, -0.1],
			[0.2, 0.1, -0.5, 0.4],
		];
		for (i, f) in feats.iter().enumerate() {
			g.add_node(&format!("n{i}"), f.to_vec()).unwrap();
		}
		g.add_edge("n0", "n1").unwrap();
		g.add_edge("n1", "n2").unwrap();
		g.add_edge("n2", "n0").unwrap();
		g.add_self_loops();
		let x = g.feature_matrix();
		(g, x)
	}

	#[test]
	fn forward_projects_to_out_layer_width_and_is_finite() {
		let (g, x) = tiny_graph();
		let mut rng = StdRng::seed_from_u64(3);
		let mut model = Model::new(
			vec![Box::new(GCNLayer::with_rng(4, 3, None, false, &mut rng))],
			None,
		);
		let out = model.forward(&g, &x);
		assert_eq!(out.rows, g.num_nodes(), "one row per node");
		assert_eq!(out.cols, 3, "width equals the layer's out_features");
		assert!(out.data.iter().all(|v| v.is_finite()), "no NaN/inf");
	}
}
