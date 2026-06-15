use crate::gnn::activation::Activation;
use crate::gnn::backward::{
	act_deriv_mul, l2_norm_backward, l2_normalize_rows, BackwardGraphLayer, GraphLayer,
};
use crate::gnn::dropout::Dropout;
use crate::gnn::graph::Graph;
use crate::gnn::layer::{Backward, Layer, LinearLayer};
use crate::gnn::message::AggregateFunc;
use crate::gnn::norm::LayerNorm;
use crate::gnn::tensor::Tensor;

pub struct SAGELayer {
	pub linear: LinearLayer,
	pub agg_func: AggregateFunc,
	pub norm: Option<LayerNorm>,
	pub drop: Option<Dropout>,
	pub act: Option<Activation>,
	pub l2_norm: bool,
	pub in_features: usize,
	last_concats: Option<Tensor>,
	last_nbr_idxs: Vec<Vec<usize>>,
	last_pre_act: Option<Tensor>,
	last_l2_in: Option<Tensor>,
}

impl SAGELayer {
	pub fn new(
		in_features: usize,
		out_features: usize,
		agg: AggregateFunc,
		act: Option<Activation>,
		l2_norm: bool,
		layer_norm: bool,
		drop_rate: f64,
	) -> Self {
		Self {
			linear: LinearLayer::new(2 * in_features, out_features),
			agg_func: agg,
			norm: if layer_norm {
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
			l2_norm,
			in_features,
			last_concats: None,
			last_nbr_idxs: Vec::new(),
			last_pre_act: None,
			last_l2_in: None,
		}
	}

	/// Human-readable configuration summary for diagnostics. `SAGELayer` cannot
	/// derive `Debug` (its `agg_func` is a bare fn pointer with no useful Debug),
	/// so this reports the input width, the concatenated linear-input width, and
	/// which optional stages (layer-norm, dropout, activation, L2) are enabled —
	/// enough to spot a mis-wired layer in a log line.
	pub fn describe(&self) -> String {
		format!(
			"SAGELayer {{ in_features: {}, concat_in: {}, layer_norm: {}, dropout: {}, act: {}, l2_norm: {} }}",
			self.in_features,
			2 * self.in_features,
			self.norm.is_some(),
			self.drop.is_some(),
			self.act.is_some(),
			self.l2_norm,
		)
	}
}

impl GraphLayer for SAGELayer {
	fn forward_graph(&mut self, g: &Graph, features: &Tensor) -> Tensor {
		let n = g.num_nodes();
		let inf = self.in_features;
		let mut concats = Tensor::zeros(n, 2 * inf);
		let mut nbr_idxs = vec![Vec::new(); n];
		let zero_msg = Tensor::zeros(1, inf);

		for (i, node) in g.nodes.iter().enumerate() {
			let self_feat = features.row(i);
			let neighbors = g.in_neighbors(&node.id);
			let idxs: Vec<usize> = neighbors
				.iter()
				.filter_map(|nbr| g.node_index(nbr))
				.collect();
			let messages: Vec<Tensor> = idxs.iter().map(|&idx| features.row(idx)).collect();
			nbr_idxs[i] = idxs;

			let agg = (self.agg_func)(&messages).unwrap_or_else(|| zero_msg.clone());

			concats.data[i * (2 * inf)..i * (2 * inf) + inf].copy_from_slice(&self_feat.data);
			concats.data[i * (2 * inf) + inf..(i + 1) * (2 * inf)].copy_from_slice(&agg.data);
		}
		self.last_concats = Some(concats.clone());
		self.last_nbr_idxs = nbr_idxs;

		let mut output = self.linear.forward(&concats);
		if let Some(ref mut nm) = self.norm {
			output = nm.forward(&output);
		}
		self.last_pre_act = Some(output.clone());
		if let Some(a) = self.act {
			output = output.apply(|x| a.forward(x));
		}
		if self.l2_norm {
			self.last_l2_in = Some(output.clone());
			output = l2_normalize_rows(&output);
		} else {
			self.last_l2_in = None;
		}
		if let Some(ref mut d) = self.drop {
			output = d.forward(&output);
		}
		output
	}

	fn parameters(&self) -> Vec<&Tensor> {
		let mut p = self.linear.parameters();
		if let Some(ref nm) = self.norm {
			p.extend(Layer::parameters(nm));
		}
		p
	}

	fn parameters_mut(&mut self) -> Vec<&mut Tensor> {
		let mut p = self.linear.parameters_mut();
		if let Some(ref mut nm) = self.norm {
			p.extend(Layer::parameters_mut(nm));
		}
		p
	}

	fn set_training(&mut self, training: bool) {
		if let Some(ref mut d) = self.drop {
			d.set_training(training);
		}
	}
}

impl BackwardGraphLayer for SAGELayer {
	/// Backward pass.
	///
	/// IMPORTANT: the neighbour half of `d_concat` is distributed back to each
	/// neighbour with `scale = 1/|N(i)|` — the derivative of MEAN aggregation.
	/// This is correct only when the layer was built with a mean `agg_func`. With
	/// a sum or max aggregator the forward pass differs and these neighbour
	/// gradients are wrong; this backward currently assumes mean-pool. Keep
	/// `agg_func` = mean during training, or extend this to dispatch per aggregator.
	fn backward_graph(&mut self, g: &Graph, d_out: &Tensor) -> Tensor {
		let mut grad = d_out.clone();
		if let Some(ref d) = self.drop {
			grad = d.backward(&grad);
		}
		if self.l2_norm {
			if let Some(ref l2_in) = self.last_l2_in {
				grad = l2_norm_backward(l2_in, &grad);
			}
		}
		if let Some(a) = self.act {
			let pre_act = self.last_pre_act.as_ref().unwrap();
			grad = act_deriv_mul(a, &grad, pre_act);
		}
		if let Some(ref mut nm) = self.norm {
			grad = nm.backward(&grad);
		}
		let d_concat = self.linear.backward(&grad);

		let inf = self.in_features;
		let n = g.num_nodes();
		let mut d_features = Tensor::zeros(n, inf);

		for i in 0..n {
			for d in 0..inf {
				d_features.data[i * inf + d] += d_concat.at(i, d);
			}
			let nbrs = &self.last_nbr_idxs[i];
			if nbrs.is_empty() {
				continue;
			}
			let scale = 1.0 / nbrs.len() as f64;
			for &j in nbrs {
				for d in 0..inf {
					d_features.data[j * inf + d] += d_concat.at(i, inf + d) * scale;
				}
			}
		}
		d_features
	}

	fn param_grads(&self) -> Vec<&Tensor> {
		let mut g = self.linear.param_grads();
		if let Some(ref nm) = self.norm {
			g.extend(Backward::param_grads(nm));
		}
		g
	}

	fn param_grads_mut(&mut self) -> Vec<&mut Tensor> {
		let mut g = self.linear.param_grads_mut();
		if let Some(ref mut nm) = self.norm {
			g.extend(Backward::param_grads_mut(nm));
		}
		g
	}

	fn zero_grads(&mut self) {
		self.linear.zero_grads();
		if let Some(ref mut nm) = self.norm {
			Backward::zero_grads(nm);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gnn::message::mean_aggregate;

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
		g.add_edge("n0", "n1", vec![]).unwrap();
		g.add_edge("n1", "n2", vec![]).unwrap();
		g.add_edge("n2", "n0", vec![]).unwrap();
		g.add_self_loops();
		let x = g.feature_matrix();
		(g, x)
	}

	#[test]
	fn forward_graph_output_is_n_by_out_features_and_finite() {
		let (g, x) = tiny_graph();
		let mut l = SAGELayer::new(4, 3, mean_aggregate, None, false, false, 0.0);
		let out = l.forward_graph(&g, &x);
		assert_eq!(out.rows, g.num_nodes(), "one output row per node");
		assert_eq!(out.cols, 3, "output width equals out_features");
		assert!(
			out.data.iter().all(|v| v.is_finite()),
			"no NaN/inf in output"
		);
	}

	#[test]
	fn describe_reports_enabled_stages() {
		let l = SAGELayer::new(
			4,
			3,
			mean_aggregate,
			Some(Activation::Relu),
			true,
			true,
			0.5,
		);
		let s = l.describe();
		assert!(s.contains("in_features: 4"), "{s}");
		assert!(s.contains("concat_in: 8"), "{s}");
		assert!(s.contains("layer_norm: true"), "{s}");
		assert!(s.contains("dropout: true"), "{s}");
		assert!(s.contains("act: true"), "{s}");
		assert!(s.contains("l2_norm: true"), "{s}");
	}

	#[test]
	fn describe_reports_a_bare_layer() {
		let l = SAGELayer::new(4, 3, mean_aggregate, None, false, false, 0.0);
		let s = l.describe();
		assert!(s.contains("layer_norm: false") && s.contains("dropout: false"));
		assert!(s.contains("act: false") && s.contains("l2_norm: false"));
	}
}
