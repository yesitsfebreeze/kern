use crate::gnn::graph::Graph;
use crate::gnn::model::Model;
use crate::gnn::optim::Optimizer;
use crate::gnn::tensor::Tensor;
pub type LossFunc = fn(&Tensor, &Tensor) -> f64;

pub type GradFunc = fn(&Tensor, &Tensor) -> Tensor;

pub struct TrainConfig {
	pub epochs: usize,
	pub lr: f64,
	pub log_every: usize, // 0 = never
	pub clip_norm: f64,   // 0 = no clipping
}

impl Default for TrainConfig {
	fn default() -> Self {
		Self {
			epochs: 100,
			lr: 0.01,
			log_every: 10,
			clip_norm: 0.0,
		}
	}
}

#[derive(Debug, Clone)]
pub struct EpochResult {
	pub epoch: usize,
	pub loss: f64,
}

/// The non-mutated inputs to [`train`], bundled so the entry point takes
/// `(model, optim, ctx)` instead of an eight-positional signature. Borrows the
/// graph/features/labels for the duration of training.
pub struct TrainContext<'a> {
	pub loss_fn: LossFunc,
	pub grad_fn: GradFunc,
	pub config: &'a TrainConfig,
	pub graph: &'a Graph,
	pub features: &'a Tensor,
	pub labels: &'a Tensor,
}

pub fn train(model: &mut Model, optim: &mut dyn Optimizer, ctx: &TrainContext) -> Vec<EpochResult> {
	let config = ctx.config;
	let mut results = Vec::with_capacity(config.epochs);

	for epoch in 1..=config.epochs {
		model.zero_grads();

		let predicted = model.forward(ctx.graph, ctx.features);
		let loss = (ctx.loss_fn)(&predicted, ctx.labels);

		let d_out = (ctx.grad_fn)(&predicted, ctx.labels);
		model.backward(ctx.graph, &d_out);

		if config.clip_norm > 0.0 {
			clip_gradients(model, config.clip_norm);
		}

		{
			let grads: Vec<Tensor> = model.param_grads().iter().map(|t| (*t).clone()).collect();
			let grad_refs: Vec<&Tensor> = grads.iter().collect();
			let mut params = model.parameters_mut();
			optim.step(&mut params, &grad_refs);
		}

		results.push(EpochResult { epoch, loss });
	}
	results
}

pub fn clip_gradients(model: &mut Model, max_norm: f64) {
	let norm_sq: f64 = model
		.param_grads()
		.iter()
		.map(|g| g.data.iter().map(|v| v * v).sum::<f64>())
		.sum();
	let norm = norm_sq.sqrt();
	if norm == 0.0 || norm <= max_norm {
		return;
	}
	let scale = max_norm / norm;
	for g in model.param_grads_mut() {
		g.scale_inplace(scale);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gnn::gcn::GCNLayer;
	use crate::gnn::optim::SGD;
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
		g.add_edge("n0", "n1", vec![]).unwrap();
		g.add_edge("n1", "n2", vec![]).unwrap();
		g.add_edge("n2", "n0", vec![]).unwrap();
		g.add_self_loops();
		let x = g.feature_matrix();
		(g, x)
	}

	fn mse(pred: &Tensor, label: &Tensor) -> f64 {
		let n = pred.data.len() as f64;
		pred
			.data
			.iter()
			.zip(&label.data)
			.map(|(p, y)| (p - y) * (p - y))
			.sum::<f64>()
			/ n
	}
	fn mse_grad(pred: &Tensor, label: &Tensor) -> Tensor {
		let n = pred.data.len() as f64;
		let data: Vec<f64> = pred
			.data
			.iter()
			.zip(&label.data)
			.map(|(p, y)| 2.0 * (p - y) / n)
			.collect();
		Tensor::new(pred.rows, pred.cols, data).unwrap()
	}

	fn grad_norm(model: &Model) -> f64 {
		model
			.param_grads()
			.iter()
			.map(|g| g.data.iter().map(|v| v * v).sum::<f64>())
			.sum::<f64>()
			.sqrt()
	}

	#[test]
	fn train_drives_a_linear_gcn_toward_zero_labels() {
		let (g, x) = tiny_graph();
		let mut rng = StdRng::seed_from_u64(11);
		// Linear GCN (no activation) -> MSE toward zero is convex, so GD must reduce loss.
		let mut model = Model::new(
			vec![Box::new(GCNLayer::with_rng(
				4, 2, None, false, 0.0, &mut rng,
			))],
			None,
		);
		let labels = Tensor::zeros(3, 2);
		let cfg = TrainConfig {
			epochs: 30,
			lr: 0.1,
			log_every: 0,
			clip_norm: 0.0,
		};
		let mut optim = SGD::new(cfg.lr);
		let ctx = TrainContext {
			loss_fn: mse,
			grad_fn: mse_grad,
			config: &cfg,
			graph: &g,
			features: &x,
			labels: &labels,
		};

		let results = train(&mut model, &mut optim, &ctx);
		assert_eq!(results.len(), 30, "one EpochResult per epoch");
		assert_eq!(results[0].epoch, 1, "epochs are 1-indexed");
		assert_eq!(results[29].epoch, 30);
		assert!(
			results.last().unwrap().loss < results[0].loss,
			"loss decreases: {} -> {}",
			results[0].loss,
			results.last().unwrap().loss
		);
	}

	#[test]
	fn clip_gradients_scales_norm_down_to_max() {
		let (g, x) = tiny_graph();
		let mut rng = StdRng::seed_from_u64(5);
		let mut model = Model::new(
			vec![Box::new(GCNLayer::with_rng(
				4, 2, None, false, 0.0, &mut rng,
			))],
			None,
		);
		let pred = model.forward(&g, &x);
		let labels = Tensor::zeros(pred.rows, pred.cols);
		model.zero_grads();
		let d = mse_grad(&pred, &labels);
		model.backward(&g, &d);

		let pre = grad_norm(&model);
		assert!(pre > 0.0, "there is a non-zero gradient to clip");
		let max_norm = pre / 2.0;
		clip_gradients(&mut model, max_norm);
		let post = grad_norm(&model);
		assert!(
			(post - max_norm).abs() < 1e-9,
			"post-clip norm equals max_norm: got {post}, want {max_norm}"
		);
	}
}
