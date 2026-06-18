use crate::gnn::activation::Activation;
use crate::gnn::dropout::Dropout;
use crate::gnn::graph::Graph;
use crate::gnn::tensor::Tensor;

/// Multiply an incoming gradient by the activation's analytic derivative,
/// evaluated at the pre-activation values. Exact (no finite-difference bias at
/// kinks) and half the activation evaluations of a central difference.
pub fn act_deriv_mul(act: Activation, d_out: &Tensor, pre_act: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(d_out.rows, d_out.cols);
	for (i, &x) in pre_act.data.iter().enumerate() {
		out.data[i] = d_out.data[i] * act.deriv(x);
	}
	out
}

/// Sum of squares of a single tensor row. Shared by the L2-norm forward and
/// backward passes so the per-row reduction lives in one place.
fn row_sum_sq(t: &Tensor, row: usize) -> f64 {
	let mut sum_sq = 0.0;
	for j in 0..t.cols {
		let v = t.at(row, j);
		sum_sq += v * v;
	}
	sum_sq
}

pub fn l2_normalize_rows(t: &Tensor) -> Tensor {
	let mut out = t.clone();
	for i in 0..t.rows {
		let sum_sq = row_sum_sq(t, i);
		if sum_sq == 0.0 {
			continue;
		}
		let inv_norm = 1.0 / sum_sq.sqrt();
		for j in 0..t.cols {
			out.set(i, j, t.at(i, j) * inv_norm);
		}
	}
	out
}

pub fn l2_norm_backward(pre_norm: &Tensor, d_out: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(pre_norm.rows, pre_norm.cols);
	for i in 0..pre_norm.rows {
		let sum_sq = row_sum_sq(pre_norm, i);
		if sum_sq == 0.0 {
			continue;
		}
		let inv_norm = 1.0 / sum_sq.sqrt();
		// Tangent-space projection: dL/dx = (d_out - x̂·(d_out·x̂)) / ‖x‖, where
		// x̂ = x/‖x‖. The dot is with the NORMALIZED row x̂ — dotting with the raw
		// `pre_norm` here drops a 1/‖x‖ factor and inflates the gradient (verified
		// against a numeric central difference).
		let mut dot_val = 0.0;
		for j in 0..pre_norm.cols {
			let x_hat = pre_norm.at(i, j) * inv_norm;
			dot_val += d_out.at(i, j) * x_hat;
		}
		for j in 0..pre_norm.cols {
			let x_hat = pre_norm.at(i, j) * inv_norm;
			out.set(i, j, (d_out.at(i, j) - x_hat * dot_val) * inv_norm);
		}
	}
	out
}

pub trait GraphLayer {
	fn forward_graph(&mut self, g: &Graph, features: &Tensor) -> Tensor;
	fn parameters(&self) -> Vec<&Tensor>;
	fn parameters_mut(&mut self) -> Vec<&mut Tensor>;

	/// The layer's dropout, if it has one. Backs the default `set_training`.
	fn dropout_mut(&mut self) -> Option<&mut Dropout>;

	/// Switch train/eval mode. Default: flip the layer's dropout (if any) —
	/// the only train-mode-sensitive component these layers carry.
	fn set_training(&mut self, training: bool) {
		if let Some(d) = self.dropout_mut() {
			d.set_training(training);
		}
	}
}

pub trait BackwardGraphLayer: GraphLayer {
	fn backward_graph(&mut self, g: &Graph, d_out: &Tensor) -> Tensor;
	fn param_grads(&self) -> Vec<&Tensor>;
	fn param_grads_mut(&mut self) -> Vec<&mut Tensor>;
	fn zero_grads(&mut self);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn relu_backward_is_exact_no_kink_bias() {
		// Pre-activations straddling zero, incoming grad all 1.0. With the old
		// central-difference, x=+/-1e-6 leaked ~0.5; the analytic derivative
		// gates exactly: 0 where x<=0, pass-through where x>0.
		let pre = Tensor {
			data: vec![-2.0, -1e-6, 0.0, 1e-6, 3.0],
			rows: 1,
			cols: 5,
		};
		let d_out = Tensor {
			data: vec![1.0; 5],
			rows: 1,
			cols: 5,
		};
		let g = act_deriv_mul(Activation::Relu, &d_out, &pre);
		assert_eq!(g.data, vec![0.0, 0.0, 0.0, 1.0, 1.0]);
	}

	#[test]
	fn backward_scales_incoming_gradient_by_deriv() {
		let pre = Tensor {
			data: vec![1.0, -1.0],
			rows: 1,
			cols: 2,
		};
		let d_out = Tensor {
			data: vec![0.5, 0.5],
			rows: 1,
			cols: 2,
		};
		let g = act_deriv_mul(Activation::LeakyRelu(0.2), &d_out, &pre);
		assert_eq!(g.data, vec![0.5, 0.1]); // 0.5*1, 0.5*0.2
	}

	#[test]
	fn l2_norm_backward_matches_numeric_gradient() {
		// loss = sum(l2_normalize_rows(x)); d_out is all-ones. The analytic
		// backward must match a central finite-difference of the loss w.r.t. each
		// input element (the projection `(I - x̂x̂ᵀ)/‖x‖` is easy to get subtly
		// wrong, so pin it against numerics).
		let x = Tensor {
			data: vec![0.5, -0.2, 0.1, -0.4, 0.6, 0.2],
			rows: 2,
			cols: 3,
		};
		let d_out = Tensor {
			data: vec![1.0; 6],
			rows: 2,
			cols: 3,
		};
		let analytic = l2_norm_backward(&x, &d_out);
		let loss = |t: &Tensor| -> f64 { l2_normalize_rows(t).data.iter().sum() };
		const H: f64 = 1e-6;
		for idx in 0..x.data.len() {
			let mut xp = x.clone();
			xp.data[idx] += H;
			let mut xm = x.clone();
			xm.data[idx] -= H;
			let num = (loss(&xp) - loss(&xm)) / (2.0 * H);
			let den = 1.0_f64.max(analytic.data[idx].abs()).max(num.abs());
			assert!(
				(analytic.data[idx] - num).abs() / den < 1e-4,
				"grad[{idx}]: analytic {} vs numeric {num}",
				analytic.data[idx]
			);
		}
	}

	#[test]
	fn l2_norm_backward_zero_row_yields_zero_grad() {
		// A zero row has no defined direction; forward and backward both skip it,
		// so its gradient stays zero (no NaN from a 1/0 norm).
		let x = Tensor {
			data: vec![0.0, 0.0, 3.0, 4.0],
			rows: 2,
			cols: 2,
		};
		let d_out = Tensor {
			data: vec![1.0; 4],
			rows: 2,
			cols: 2,
		};
		let g = l2_norm_backward(&x, &d_out);
		assert_eq!(&g.data[0..2], &[0.0, 0.0], "zero row -> zero grad, no NaN");
		assert!(
			g.data[2..].iter().all(|v| v.is_finite()),
			"non-zero row grad is finite"
		);
	}
}

/// Numeric gradient checks for the GNN math-critical paths: layer backward
/// passes (vs central finite differences) and core tensor ops. Purely additive
/// coverage — no production behaviour change.
#[cfg(test)]
mod gnn_math_tests {
	use crate::gnn::activation::Activation;
	use crate::gnn::backward::BackwardGraphLayer;
	use crate::gnn::gcn::GCNLayer;
	use crate::gnn::graph::Graph;
	use crate::gnn::tensor::Tensor;
	use rand::SeedableRng;

	/// Fixed 3-node ring (with self-loops) and its feature matrix.
	fn tiny_graph() -> (Graph, Tensor) {
		let feats = [
			[0.5, -0.2, 0.1, 0.3],
			[-0.4, 0.6, 0.2, -0.1],
			[0.2, 0.1, -0.5, 0.4],
		];
		let mut g = Graph::new();
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

	/// Compare analytic param gradients (backward with d_out = ones, so the
	/// scalar loss is sum(output)) against central finite differences over
	/// every parameter element. Init-agnostic: holds for any weights.
	fn assert_grad_matches_numeric(layer: &mut dyn BackwardGraphLayer, g: &Graph, x: &Tensor) {
		const H: f64 = 1e-6;
		let out = layer.forward_graph(g, x);
		let d_out = Tensor::ones(out.rows, out.cols);
		layer.zero_grads();
		layer.backward_graph(g, &d_out);
		let analytic: Vec<f64> = layer
			.param_grads()
			.iter()
			.flat_map(|t| t.data.clone())
			.collect();

		let lens: Vec<usize> = layer.parameters().iter().map(|t| t.data.len()).collect();
		let mut numeric = Vec::with_capacity(analytic.len());
		for (pi, &len) in lens.iter().enumerate() {
			for ei in 0..len {
				layer.parameters_mut()[pi].data[ei] += H;
				let lp = layer.forward_graph(g, x).sum_all();
				layer.parameters_mut()[pi].data[ei] -= 2.0 * H;
				let lm = layer.forward_graph(g, x).sum_all();
				layer.parameters_mut()[pi].data[ei] += H; // restore
				numeric.push((lp - lm) / (2.0 * H));
			}
		}

		assert_eq!(analytic.len(), numeric.len(), "grad length mismatch");
		for (i, (a, n)) in analytic.iter().zip(&numeric).enumerate() {
			let denom = 1.0_f64.max(a.abs()).max(n.abs());
			assert!(
				(a - n).abs() / denom < 1e-4,
				"param grad[{i}]: analytic {a} vs numeric {n}"
			);
		}
	}

	/// Compare the analytic INPUT gradient (the tensor `backward_graph` returns —
	/// dL/d(input features) for the scalar loss `sum(output)`) against central
	/// finite differences over every input element. This is the gradient that
	/// chains to the PREVIOUS layer in a stacked model (`Model::backward` feeds it
	/// back as the next layer's `d_out`). The param-gradient check above never
	/// exercises it, and every `model.rs` test is single-layer, so without this the
	/// layer-to-layer gradient flow is unverified.
	fn assert_input_grad_matches_numeric(layer: &mut dyn BackwardGraphLayer, g: &Graph, x: &Tensor) {
		const H: f64 = 1e-6;
		let out = layer.forward_graph(g, x);
		let d_out = Tensor::ones(out.rows, out.cols);
		layer.zero_grads();
		let analytic = layer.backward_graph(g, &d_out);
		assert_eq!(
			analytic.shape(),
			x.shape(),
			"d_input shape must match features"
		);

		let mut numeric = Vec::with_capacity(x.data.len());
		for ei in 0..x.data.len() {
			let mut xp = x.clone();
			xp.data[ei] += H;
			let lp = layer.forward_graph(g, &xp).sum_all();
			let mut xm = x.clone();
			xm.data[ei] -= H;
			let lm = layer.forward_graph(g, &xm).sum_all();
			numeric.push((lp - lm) / (2.0 * H));
		}
		for (i, (a, n)) in analytic.data.iter().zip(&numeric).enumerate() {
			let denom = 1.0_f64.max(a.abs()).max(n.abs());
			assert!(
				(a - n).abs() / denom < 1e-4,
				"input grad[{i}]: analytic {a} vs numeric {n}"
			);
		}
	}

	#[test]
	fn gcn_linear_input_grad_matches_numeric() {
		// d_input = Aᵀ·(d_out·Wᵀ): the layer-chaining gradient, previously unchecked.
		let (g, x) = tiny_graph();
		let mut rng = rand::rngs::StdRng::seed_from_u64(23);
		let mut l = GCNLayer::with_rng(4, 3, None, false, 0.0, &mut rng);
		assert_input_grad_matches_numeric(&mut l, &g, &x);
	}

	#[test]
	fn gcn_relu_input_grad_matches_numeric() {
		let (g, x) = tiny_graph();
		let mut rng = rand::rngs::StdRng::seed_from_u64(29);
		let mut l = GCNLayer::with_rng(4, 3, Some(Activation::Relu), false, 0.0, &mut rng);
		assert_input_grad_matches_numeric(&mut l, &g, &x);
	}

	#[test]
	fn gcn_linear_backward_matches_numeric() {
		let (g, x) = tiny_graph();
		let mut rng = rand::rngs::StdRng::seed_from_u64(7);
		let mut l = GCNLayer::with_rng(4, 3, None, false, 0.0, &mut rng);
		assert_grad_matches_numeric(&mut l, &g, &x);
	}

	#[test]
	fn gcn_relu_backward_matches_numeric() {
		// End-to-end check of the analytic ReLU derivative (see the
		// finite-difference-derivative fix).
		let (g, x) = tiny_graph();
		let mut rng = rand::rngs::StdRng::seed_from_u64(11);
		let mut l = GCNLayer::with_rng(4, 3, Some(Activation::Relu), false, 0.0, &mut rng);
		assert_grad_matches_numeric(&mut l, &g, &x);
	}

	#[test]
	fn matmul_and_transpose_are_correct() {
		let a = Tensor::new(2, 2, vec![1.0, 2.0, 3.0, 4.0]).unwrap();
		let b = Tensor::new(2, 2, vec![5.0, 6.0, 7.0, 8.0]).unwrap();
		let c = a.matmul(&b).unwrap();
		assert_eq!(c.data, vec![19.0, 22.0, 43.0, 50.0]);
		let at = a.transpose();
		assert_eq!(at.data, vec![1.0, 3.0, 2.0, 4.0]);
	}
}
