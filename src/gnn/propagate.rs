use std::collections::{HashMap, HashSet};

use crate::gnn::activation::Activation;
use crate::gnn::backward::BackwardGraphLayer;
use crate::gnn::gcn::GCNLayer;
use crate::gnn::graph::Graph;
use crate::gnn::loss::{link_prediction_grad, link_prediction_loss};
use crate::gnn::model::Model;
use crate::gnn::optim::Adam;
use crate::gnn::persist::{marshal_weights, unmarshal_weights};
use crate::gnn::tensor::Tensor;

/// Single source of truth for the GnnConfig defaults — both [`GnnConfig::defaults`]
/// and the serde `crate::config::GnnConfig` must read them from here, never re-literal.
pub const DEFAULT_SELF_WEIGHT: f64 = 0.6;
pub const DEFAULT_MIN_WEIGHT: f64 = 0.01;
pub const DEFAULT_MIN_THOUGHTS: usize = 128;
pub const DEFAULT_TRAIN_EPOCHS: usize = 24;
pub const DEFAULT_TRAIN_LEARNING_RATE: f64 = 0.01;

#[derive(Debug, Clone, Copy)]
pub struct GnnConfig {
	pub self_weight: f64,
	pub min_weight: f64,
	pub min_thoughts: usize,
	pub train_epochs: usize,
	pub train_learning_rate: f64,
}

impl GnnConfig {
	pub fn defaults() -> Self {
		Self {
			self_weight: DEFAULT_SELF_WEIGHT,
			min_weight: DEFAULT_MIN_WEIGHT,
			min_thoughts: DEFAULT_MIN_THOUGHTS,
			train_epochs: DEFAULT_TRAIN_EPOCHS,
			train_learning_rate: DEFAULT_TRAIN_LEARNING_RATE,
		}
	}
}

impl Default for GnnConfig {
	fn default() -> Self {
		Self::defaults()
	}
}

pub struct GnnSnapshot {
	pub ids: Vec<String>,
	pub features: Tensor,
	pub graph: Graph,
	pub pos_edges: Vec<[usize; 2]>,
	pub weights: Vec<u8>,
}

pub struct PropagationResult {
	pub updates: HashMap<String, Vec<f64>>,
	pub weights: Vec<u8>,
}

pub fn run_learned_propagation(
	snap: &GnnSnapshot,
	cfg: &GnnConfig,
) -> Result<PropagationResult, String> {
	if snap.ids.is_empty() {
		return Err("empty snapshot".into());
	}
	let dim = snap.features.cols;
	let hidden = (dim / 2).clamp(16, 256);

	let neg_edges = sample_negative_edges(snap.ids.len(), &snap.pos_edges, snap.pos_edges.len());
	if neg_edges.is_empty() {
		return Err("could not sample negative edges".into());
	}

	let l1: Box<dyn BackwardGraphLayer> = Box::new(GCNLayer::new(
		dim,
		hidden,
		Some(Activation::Relu),
		true,
		0.0,
	));
	let l2: Box<dyn BackwardGraphLayer> = Box::new(GCNLayer::new(hidden, dim, None, false, 0.0));
	let mut model = Model::new(vec![l1, l2], None);

	if !snap.weights.is_empty() {
		if let Err(e) = unmarshal_weights(&mut model, &snap.weights) {
			tracing::error!(error = %e, "GNN weight load failed; cold-starting from fresh weights");
		}
	}

	let pos = snap.pos_edges.clone();
	let neg = neg_edges.clone();
	let mut optim = Adam::new(cfg.train_learning_rate);

	for _epoch in 0..cfg.train_epochs {
		model.zero_grads();
		let predicted = model.forward(&snap.graph, &snap.features);
		let _loss = link_prediction_loss(&predicted, &pos, &neg);
		let d_out = link_prediction_grad(&predicted, &pos, &neg);
		model.backward(&snap.graph, &d_out);

		let grads: Vec<Tensor> = model.param_grads().iter().map(|t| (*t).clone()).collect();
		let grad_refs: Vec<&Tensor> = grads.iter().collect();
		let mut params = model.parameters_mut();
		use crate::gnn::optim::Optimizer;
		optim.step(&mut params, &grad_refs);
	}

	model.set_training(false);
	let emb = model.forward(&snap.graph, &snap.features);
	let mut updates = HashMap::new();

	for (i, id) in snap.ids.iter().enumerate() {
		let row = emb.row(i);
		if row.data.len() != dim {
			continue;
		}
		if has_nan_or_inf(&row.data) {
			continue;
		}
		let mut result = vec![0.0; dim];
		for (d, slot) in result.iter_mut().enumerate().take(dim) {
			*slot = cfg.self_weight * snap.features.at(i, d) + (1.0 - cfg.self_weight) * row.data[d];
		}
		updates.insert(id.clone(), gnn_normalize(&result));
	}

	let weights = marshal_weights(&model).map_err(|e| format!("marshal weights: {e}"))?;
	Ok(PropagationResult { updates, weights })
}

pub fn sample_negative_edges(n: usize, pos_edges: &[[usize; 2]], want: usize) -> Vec<[usize; 2]> {
	if n < 2 || want == 0 {
		return Vec::new();
	}
	let mut pos_set = HashSet::new();
	for e in pos_edges {
		let (a, b) = if e[0] < e[1] {
			(e[0], e[1])
		} else {
			(e[1], e[0])
		};
		pos_set.insert((a, b));
	}
	let max_pairs = n * (n - 1) / 2;
	let max_neg = max_pairs.saturating_sub(pos_set.len());
	if max_neg == 0 {
		return Vec::new();
	}
	let want = want.min(max_neg);

	use rand::RngExt;
	let mut rng = rand::rng();
	let mut neg_set = HashSet::new();
	let mut neg = Vec::with_capacity(want);
	let limit = want * 30;
	let mut attempts = 0;
	while neg.len() < want && attempts < limit {
		attempts += 1;
		let a = rng.random_range(0..n);
		let b = rng.random_range(0..n);
		if a == b {
			continue;
		}
		let (lo, hi) = if a < b { (a, b) } else { (b, a) };
		if pos_set.contains(&(lo, hi)) || neg_set.contains(&(lo, hi)) {
			continue;
		}
		neg_set.insert((lo, hi));
		neg.push([lo, hi]);
	}
	neg
}

pub fn gnn_normalize(v: &[f64]) -> Vec<f64> {
	let norm_sq: f64 = v.iter().map(|x| x * x).sum();
	if norm_sq == 0.0 {
		return v.to_vec();
	}
	let inv = 1.0 / norm_sq.sqrt();
	v.iter().map(|x| x * inv).collect()
}

fn has_nan_or_inf(v: &[f64]) -> bool {
	v.iter().any(|x| x.is_nan() || x.is_infinite())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tiny_snapshot(n: usize, dim: usize) -> GnnSnapshot {
		let mut graph = Graph::new();
		for i in 0..n {
			let feats: Vec<f64> = (0..dim).map(|d| ((i + d) as f64).sin()).collect();
			graph.add_node(&format!("n{i}"), feats).unwrap();
		}
		let mut pos_edges = Vec::new();
		for i in 0..n - 1 {
			graph
				.add_edge(&format!("n{i}"), &format!("n{}", i + 1), vec![1.0])
				.unwrap();
			pos_edges.push([i, i + 1]);
		}
		let data: Vec<f64> = (0..n * dim).map(|k| ((k as f64) * 0.1).cos()).collect();
		GnnSnapshot {
			ids: (0..n).map(|i| format!("n{i}")).collect(),
			features: Tensor::new(n, dim, data).unwrap(),
			graph,
			pos_edges,
			weights: Vec::new(),
		}
	}

	#[test]
	fn empty_snapshot_is_an_error() {
		let snap = GnnSnapshot {
			ids: Vec::new(),
			features: Tensor::zeros(0, 0),
			graph: Graph::new(),
			pos_edges: Vec::new(),
			weights: Vec::new(),
		};
		let err = match run_learned_propagation(&snap, &GnnConfig::defaults()) {
			Err(e) => e,
			Ok(_) => panic!("expected error for empty snapshot"),
		};
		assert_eq!(err, "empty snapshot");
	}

	#[test]
	fn happy_path_returns_finite_updates_and_weights() {
		let dim = 8;
		let snap = tiny_snapshot(6, dim);
		let cfg = GnnConfig {
			train_epochs: 3,
			..GnnConfig::defaults()
		};
		let result = run_learned_propagation(&snap, &cfg).unwrap();

		assert_eq!(result.updates.len(), snap.ids.len());
		assert!(!result.weights.is_empty(), "weights should be marshalled");
		for id in &snap.ids {
			let v = result.updates.get(id).expect("every id has an update");
			assert_eq!(v.len(), dim);
			assert!(v.iter().all(|x| x.is_finite()), "updates must be finite");
		}
	}

	#[test]
	fn sample_negative_edges_avoids_positives_and_self_loops() {
		let pos = vec![[0, 1], [1, 2]];
		let neg = sample_negative_edges(5, &pos, 4);
		for e in &neg {
			assert_ne!(e[0], e[1], "no self loops");
			let (lo, hi) = if e[0] < e[1] {
				(e[0], e[1])
			} else {
				(e[1], e[0])
			};
			assert!(
				!pos.contains(&[lo, hi]),
				"negative edge must not be a positive edge"
			);
		}
	}
}
