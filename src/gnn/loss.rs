use crate::gnn::activation::{log_softmax, sigmoid, softmax};
use crate::gnn::tensor::Tensor;

pub fn mse_loss(predicted: &Tensor, target: &Tensor) -> f64 {
	let diff = predicted.sub(target).expect("mse shape");
	diff.data.iter().map(|v| v * v).sum::<f64>() / diff.data.len() as f64
}

pub fn mse_grad(predicted: &Tensor, target: &Tensor) -> Tensor {
	let mut diff = predicted.sub(target).expect("mse_grad shape");
	let scale = 2.0 / diff.data.len() as f64;
	diff.scale_inplace(scale);
	diff
}

/// Resolve a float label to a valid class index in `[0, num_classes)`.
///
/// Returns `None` for non-finite, negative, or out-of-range labels so callers
/// can skip the row, instead of (a) panicking on a slice out-of-bounds when the
/// label is too large or (b) silently mapping a `NaN`/negative label to class 0
/// via a raw `as usize` cast (`NaN as usize == 0`, negatives saturate to 0).
fn class_index(label: f64, num_classes: usize) -> Option<usize> {
	if !label.is_finite() || label < 0.0 {
		return None;
	}
	let idx = label as usize;
	(idx < num_classes).then_some(idx)
}

pub fn cross_entropy_loss(predicted: &Tensor, target: &Tensor) -> f64 {
	let log_probs = log_softmax(predicted);
	let n = predicted.rows;
	let mut loss = 0.0;
	for i in 0..n {
		let Some(class_idx) = class_index(target.at(i, 0), predicted.cols) else {
			tracing::warn!(target: "kern.gnn", row = i, label = target.at(i, 0), "invalid class label; skipping row in cross_entropy_loss");
			continue;
		};
		loss -= log_probs.at(i, class_idx);
	}
	loss / n as f64
}

pub fn cross_entropy_grad(predicted: &Tensor, target: &Tensor) -> Tensor {
	let mut probs = softmax(predicted);
	let n = predicted.rows;
	for i in 0..n {
		match class_index(target.at(i, 0), predicted.cols) {
			Some(class_idx) => probs.set(i, class_idx, probs.at(i, class_idx) - 1.0),
			None => {
				// Invalid label: emit no gradient signal for this row rather
				// than corrupting it with an un-subtracted softmax.
				tracing::warn!(target: "kern.gnn", row = i, label = target.at(i, 0), "invalid class label; zeroing grad row in cross_entropy_grad");
				for j in 0..probs.cols {
					probs.set(i, j, 0.0);
				}
			}
		}
	}
	let scale = 1.0 / n as f64;
	probs.scale_inplace(scale);
	probs
}

pub fn nll_loss(predicted: &Tensor, target: &Tensor) -> f64 {
	let n = predicted.rows;
	let mut loss = 0.0;
	for i in 0..n {
		let Some(class_idx) = class_index(target.at(i, 0), predicted.cols) else {
			tracing::warn!(target: "kern.gnn", row = i, label = target.at(i, 0), "invalid class label; skipping row in nll_loss");
			continue;
		};
		loss -= predicted.at(i, class_idx);
	}
	loss / n as f64
}

pub fn accuracy(predicted: &Tensor, target: &Tensor) -> f64 {
	let mut correct = 0;
	for i in 0..predicted.rows {
		let pred = predicted.max_in_row(i);
		// An invalid label can never match the argmax -> counts as incorrect.
		if class_index(target.at(i, 0).round(), predicted.cols) == Some(pred) {
			correct += 1;
		}
	}
	correct as f64 / predicted.rows as f64
}

/// Gradient of [`nll_loss`] w.r.t. `predicted`. NLL on raw scores is linear in
/// the true-class score (`-predicted[i, class]`), so the gradient is `-1/n` at
/// each row's true class and `0` elsewhere. Invalid labels contribute no
/// gradient (the row stays zero), matching `nll_loss`'s row-skip behaviour.
pub fn nll_grad(predicted: &Tensor, target: &Tensor) -> Tensor {
	let (n, cols) = (predicted.rows, predicted.cols);
	let mut grad = Tensor::zeros(n, cols);
	let scale = 1.0 / n as f64;
	for i in 0..n {
		let Some(class_idx) = class_index(target.at(i, 0), cols) else {
			tracing::warn!(target: "kern.gnn", row = i, label = target.at(i, 0), "invalid class label; zeroing grad row in nll_grad");
			continue;
		};
		grad.set(i, class_idx, -scale);
	}
	grad
}

fn row_dot(t: &Tensor, i: usize, j: usize) -> f64 {
	let d = t.cols;
	let mut sum = 0.0;
	for k in 0..d {
		sum += t.at(i, k) * t.at(j, k);
	}
	sum
}

pub fn link_prediction_loss(
	embeddings: &Tensor,
	pos_edges: &[[usize; 2]],
	neg_edges: &[[usize; 2]],
) -> f64 {
	let total = pos_edges.len() + neg_edges.len();
	if total == 0 {
		return 0.0;
	}
	let mut loss = 0.0;
	for e in pos_edges {
		let dot = row_dot(embeddings, e[0], e[1]);
		loss -= (sigmoid(dot) + 1e-10).ln();
	}
	for e in neg_edges {
		let dot = row_dot(embeddings, e[0], e[1]);
		loss -= (1.0 - sigmoid(dot) + 1e-10).ln();
	}
	loss / total as f64
}

pub fn link_prediction_grad(
	embeddings: &Tensor,
	pos_edges: &[[usize; 2]],
	neg_edges: &[[usize; 2]],
) -> Tensor {
	let (n, d) = (embeddings.rows, embeddings.cols);
	let total = pos_edges.len() + neg_edges.len();
	if total == 0 {
		return Tensor::zeros(n, d);
	}
	let scale = 1.0 / total as f64;
	let mut grad = Tensor::zeros(n, d);

	for e in pos_edges {
		let (u, v) = (e[0], e[1]);
		let dot = row_dot(embeddings, u, v);
		let s = sigmoid(dot) - 1.0;
		for j in 0..d {
			grad.data[u * d + j] += scale * s * embeddings.at(v, j);
			grad.data[v * d + j] += scale * s * embeddings.at(u, j);
		}
	}
	for e in neg_edges {
		let (u, v) = (e[0], e[1]);
		let dot = row_dot(embeddings, u, v);
		let s = sigmoid(dot);
		for j in 0..d {
			grad.data[u * d + j] += scale * s * embeddings.at(v, j);
			grad.data[v * d + j] += scale * s * embeddings.at(u, j);
		}
	}
	grad
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn class_index_validates_range_and_finiteness() {
		assert_eq!(class_index(0.0, 3), Some(0));
		assert_eq!(class_index(2.0, 3), Some(2));
		assert_eq!(class_index(3.0, 3), None); // out of range
		assert_eq!(class_index(-1.0, 3), None); // negative
		assert_eq!(class_index(f64::NAN, 3), None);
		assert_eq!(class_index(f64::INFINITY, 3), None);
	}

	#[test]
	fn cross_entropy_loss_skips_out_of_range_label_no_panic() {
		let predicted = Tensor::new(1, 3, vec![0.1, 0.2, 0.7]).unwrap();
		let target = Tensor::new(1, 1, vec![5.0]).unwrap(); // class 5 of 3
		let loss = cross_entropy_loss(&predicted, &target);
		// Row skipped -> 0, NOT attributed to class 0 (which would be > 0).
		assert_eq!(loss, 0.0);
	}

	#[test]
	fn cross_entropy_loss_nan_label_not_treated_as_class_zero() {
		let predicted = Tensor::new(1, 3, vec![5.0, 0.0, 0.0]).unwrap();
		let nan_target = Tensor::new(1, 1, vec![f64::NAN]).unwrap();
		assert_eq!(cross_entropy_loss(&predicted, &nan_target), 0.0);
		// A valid class-0 label DOES produce a positive loss — proving the NaN
		// case above was genuinely skipped, not silently mapped to class 0.
		let valid = Tensor::new(1, 1, vec![0.0]).unwrap();
		assert!(cross_entropy_loss(&predicted, &valid) > 0.0);
	}

	#[test]
	fn cross_entropy_grad_zeroes_invalid_row_no_panic() {
		let predicted = Tensor::new(1, 3, vec![0.1, 0.2, 0.7]).unwrap();
		let target = Tensor::new(1, 1, vec![9.0]).unwrap();
		let grad = cross_entropy_grad(&predicted, &target);
		assert!(
			grad.data.iter().all(|&v| v == 0.0),
			"invalid row grad must be zero"
		);
	}

	#[test]
	fn accuracy_counts_invalid_label_as_incorrect() {
		// argmax of row is class 2; label 9 is out of range -> incorrect.
		let predicted = Tensor::new(1, 3, vec![0.1, 0.2, 0.7]).unwrap();
		let target = Tensor::new(1, 1, vec![9.0]).unwrap();
		assert_eq!(accuracy(&predicted, &target), 0.0);
		// Correct label scores 1.0.
		let good = Tensor::new(1, 1, vec![2.0]).unwrap();
		assert_eq!(accuracy(&predicted, &good), 1.0);
	}

	// ── nll_grad ────────────────────────────────────────────────────────────

	#[test]
	fn nll_grad_is_minus_one_over_n_at_true_class_only() {
		let predicted = Tensor::new(2, 3, vec![0.1, 0.2, 0.7, 0.3, 0.3, 0.4]).unwrap();
		let target = Tensor::new(2, 1, vec![2.0, 0.0]).unwrap();
		let g = nll_grad(&predicted, &target); // n=2 -> -1/n = -0.5
		assert_eq!(g.at(0, 2), -0.5);
		assert_eq!(g.at(1, 0), -0.5);
		// Every other cell is zero.
		assert_eq!(g.at(0, 0) + g.at(0, 1) + g.at(1, 1) + g.at(1, 2), 0.0);
	}

	#[test]
	fn nll_grad_invalid_label_leaves_row_zero() {
		let predicted = Tensor::new(1, 3, vec![0.1, 0.2, 0.7]).unwrap();
		let target = Tensor::new(1, 1, vec![9.0]).unwrap();
		assert!(nll_grad(&predicted, &target).data.iter().all(|&v| v == 0.0));
	}

	// ── link prediction ─────────────────────────────────────────────────────

	#[test]
	fn link_prediction_empty_edges_is_zero_loss_and_grad() {
		let emb = Tensor::new(3, 2, vec![1.0, 0.0, 0.0, 1.0, 1.0, 1.0]).unwrap();
		assert_eq!(link_prediction_loss(&emb, &[], &[]), 0.0);
		let g = link_prediction_grad(&emb, &[], &[]);
		assert_eq!((g.rows, g.cols), (3, 2));
		assert!(g.data.iter().all(|&v| v == 0.0));
	}

	#[test]
	fn link_prediction_aligned_positive_edge_has_lower_loss_than_opposed() {
		let aligned = Tensor::new(2, 2, vec![3.0, 0.0, 3.0, 0.0]).unwrap(); // dot +9
		let opposed = Tensor::new(2, 2, vec![3.0, 0.0, -3.0, 0.0]).unwrap(); // dot -9
		let pos = [[0usize, 1usize]];
		assert!(
			link_prediction_loss(&aligned, &pos, &[]) < link_prediction_loss(&opposed, &pos, &[]),
			"a positive edge between aligned embeddings is cheaper than between opposed ones"
		);
	}

	#[test]
	fn link_prediction_grad_matches_numerical_gradient() {
		// The analytic grad must match a central finite-difference of the loss —
		// the strongest check for the file's most complex math.
		let emb = Tensor::new(3, 2, vec![0.5, -0.2, 0.1, 0.3, -0.4, 0.6]).unwrap();
		let pos = [[0usize, 1usize], [1, 2]];
		let neg = [[0usize, 2usize]];
		let analytic = link_prediction_grad(&emb, &pos, &neg);
		const H: f64 = 1e-6;
		for idx in 0..emb.data.len() {
			let mut ep = emb.clone();
			ep.data[idx] += H;
			let mut em = emb.clone();
			em.data[idx] -= H;
			let num =
				(link_prediction_loss(&ep, &pos, &neg) - link_prediction_loss(&em, &pos, &neg)) / (2.0 * H);
			let den = 1.0_f64.max(analytic.data[idx].abs()).max(num.abs());
			assert!(
				(analytic.data[idx] - num).abs() / den < 1e-4,
				"grad[{idx}]: analytic {} vs numeric {num}",
				analytic.data[idx]
			);
		}
	}
}
