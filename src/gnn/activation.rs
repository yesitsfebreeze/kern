use crate::gnn::tensor::Tensor;

#[inline]
pub fn relu(x: f64) -> f64 {
	x.max(0.0)
}

#[inline]
pub fn relu_deriv(x: f64) -> f64 {
	if x > 0.0 {
		1.0
	} else {
		0.0
	}
}

#[inline]
pub fn sigmoid(x: f64) -> f64 {
	1.0 / (1.0 + (-x).exp())
}

#[inline]
pub fn sigmoid_deriv(x: f64) -> f64 {
	let s = sigmoid(x);
	s * (1.0 - s)
}

#[inline]
pub fn tanh_act(x: f64) -> f64 {
	x.tanh()
}

#[inline]
pub fn tanh_deriv(x: f64) -> f64 {
	let t = x.tanh();
	1.0 - t * t
}

#[inline]
pub fn leaky_relu(alpha: f64, x: f64) -> f64 {
	if x > 0.0 {
		x
	} else {
		alpha * x
	}
}

#[inline]
pub fn leaky_relu_deriv(alpha: f64, x: f64) -> f64 {
	if x > 0.0 {
		1.0
	} else {
		alpha
	}
}

// GELU via the tanh approximation (the GPT/BERT formulation): the exact erf form
// has no std primitive, and this approximation is what modern GNN/Transformer
// stacks use in practice. `gelu_deriv` is the exact derivative OF THIS
// approximation, so forward/backward stay consistent.
const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4; // sqrt(2/π)
const GELU_C: f64 = 0.044_715;

#[inline]
pub fn gelu(x: f64) -> f64 {
	let inner = SQRT_2_OVER_PI * (x + GELU_C * x * x * x);
	0.5 * x * (1.0 + inner.tanh())
}

#[inline]
pub fn gelu_deriv(x: f64) -> f64 {
	let inner = SQRT_2_OVER_PI * (x + GELU_C * x * x * x);
	let t = inner.tanh();
	let sech2 = 1.0 - t * t;
	let d_inner = SQRT_2_OVER_PI * (1.0 + 3.0 * GELU_C * x * x);
	0.5 * (1.0 + t) + 0.5 * x * sech2 * d_inner
}

/// An activation function paired with its analytic derivative.
///
/// Layers store this instead of a bare `fn(f64) -> f64` so the backward pass
/// uses the exact derivative (`deriv`) rather than a finite-difference
/// approximation, which is both biased at kinks (ReLU/leaky at x≈0) and slower.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Activation {
	Relu,
	Sigmoid,
	Tanh,
	LeakyRelu(f64),
	Gelu,
}

impl Activation {
	#[inline]
	pub fn forward(self, x: f64) -> f64 {
		match self {
			Activation::Relu => relu(x),
			Activation::Sigmoid => sigmoid(x),
			Activation::Tanh => tanh_act(x),
			Activation::LeakyRelu(alpha) => leaky_relu(alpha, x),
			Activation::Gelu => gelu(x),
		}
	}

	#[inline]
	pub fn deriv(self, x: f64) -> f64 {
		match self {
			Activation::Relu => relu_deriv(x),
			Activation::Sigmoid => sigmoid_deriv(x),
			Activation::Tanh => tanh_deriv(x),
			Activation::LeakyRelu(alpha) => leaky_relu_deriv(alpha, x),
			Activation::Gelu => gelu_deriv(x),
		}
	}
}

/// Maximum value in a tensor row — the numerically-stable shift subtracted
/// before exponentiating in `softmax` / `log_softmax`.
fn row_max(t: &Tensor, row: usize) -> f64 {
	let mut max_val = f64::NEG_INFINITY;
	for j in 0..t.cols {
		let v = t.at(row, j);
		if v > max_val {
			max_val = v;
		}
	}
	max_val
}

pub fn softmax(t: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(t.rows, t.cols);
	for i in 0..t.rows {
		let max_val = row_max(t, i);
		let mut sum = 0.0;
		for j in 0..t.cols {
			let e = (t.at(i, j) - max_val).exp();
			out.set(i, j, e);
			sum += e;
		}
		for j in 0..t.cols {
			out.set(i, j, out.at(i, j) / sum);
		}
	}
	out
}

pub fn log_softmax(t: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(t.rows, t.cols);
	for i in 0..t.rows {
		let max_val = row_max(t, i);
		let mut log_sum = 0.0;
		for j in 0..t.cols {
			log_sum += (t.at(i, j) - max_val).exp();
		}
		let log_sum = max_val + log_sum.ln();
		for j in 0..t.cols {
			out.set(i, j, t.at(i, j) - log_sum);
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn relu_deriv_is_exact_at_and_near_kink() {
		// No finite-difference smear: exactly 0 for x<=0, 1 for x>0, even at
		// magnitudes a central difference (EPS 1e-5) would have blurred to ~0.5.
		assert_eq!(Activation::Relu.deriv(-2.0), 0.0);
		assert_eq!(Activation::Relu.deriv(-1e-6), 0.0);
		assert_eq!(Activation::Relu.deriv(0.0), 0.0);
		assert_eq!(Activation::Relu.deriv(1e-6), 1.0);
		assert_eq!(Activation::Relu.deriv(3.0), 1.0);
	}

	#[test]
	fn leaky_relu_deriv_is_alpha_or_one() {
		let a = Activation::LeakyRelu(0.2);
		assert_eq!(a.deriv(-5.0), 0.2);
		assert_eq!(a.deriv(5.0), 1.0);
	}

	#[test]
	fn smooth_derivs_match_central_difference() {
		// For smooth activations the analytic derivative must agree with a
		// central finite difference (the thing we replaced) to high precision.
		const H: f64 = 1e-6;
		for &act in &[Activation::Sigmoid, Activation::Tanh] {
			for &x in &[-2.3, -0.5, 0.0, 0.7, 1.9] {
				let numeric = (act.forward(x + H) - act.forward(x - H)) / (2.0 * H);
				assert!(
					(act.deriv(x) - numeric).abs() < 1e-6,
					"{act:?} at {x}: analytic {} vs numeric {numeric}",
					act.deriv(x)
				);
			}
		}
	}

	#[test]
	fn forward_dispatches_correctly() {
		assert_eq!(Activation::Relu.forward(-1.0), 0.0);
		assert_eq!(Activation::Relu.forward(2.0), 2.0);
		assert!((Activation::LeakyRelu(0.1).forward(-3.0) - (-0.3)).abs() < 1e-12);
		assert!((Activation::Tanh.forward(0.0)).abs() < 1e-12);
		// Tanh is exposed through the enum and equals the bare method.
		assert!((Activation::Tanh.forward(0.7) - 0.7_f64.tanh()).abs() < 1e-12);
	}

	#[test]
	fn gelu_basic_properties_and_exact_derivative() {
		// gelu(0)=0; large positive ~ identity; large negative ~ 0.
		assert!((Activation::Gelu.forward(0.0)).abs() < 1e-12);
		assert!((Activation::Gelu.forward(5.0) - 5.0).abs() < 0.01, "gelu(5) ~ 5");
		assert!(Activation::Gelu.forward(-5.0).abs() < 0.01, "gelu(-5) ~ 0");
		// Its analytic deriv matches a central finite difference (the deriv is the
		// exact derivative of the tanh-approx forward, so they agree tightly).
		const H: f64 = 1e-6;
		for &x in &[-2.3, -0.5, 0.0, 0.7, 1.9] {
			let numeric = (Activation::Gelu.forward(x + H) - Activation::Gelu.forward(x - H)) / (2.0 * H);
			assert!(
				(Activation::Gelu.deriv(x) - numeric).abs() < 1e-6,
				"gelu at {x}: analytic {} vs numeric {numeric}",
				Activation::Gelu.deriv(x)
			);
		}
	}

	#[test]
	fn softmax_multirow_each_row_sums_to_one_and_is_row_independent() {
		// Three rows with different scales; the row-loop must normalize each row
		// independently (a bug in the bounds would leak across rows).
		let t = Tensor::new(3, 3, vec![
			1.0, 2.0, 3.0,
			10.0, 10.0, 10.0, // uniform row -> 1/3 each
			-5.0, 0.0, 5.0,
		]).unwrap();
		let s = softmax(&t);
		for i in 0..3 {
			let row_sum: f64 = (0..3).map(|j| s.at(i, j)).sum();
			assert!((row_sum - 1.0).abs() < 1e-12, "row {i} sums to 1, got {row_sum}");
		}
		// Uniform row -> exactly 1/3 each.
		for j in 0..3 {
			assert!((s.at(1, j) - 1.0 / 3.0).abs() < 1e-12);
		}
		// Row 0 is monotonic in the logits (3 > 2 > 1).
		assert!(s.at(0, 2) > s.at(0, 1) && s.at(0, 1) > s.at(0, 0));
	}

	#[test]
	fn log_softmax_multirow_equals_log_of_softmax() {
		let t = Tensor::new(2, 3, vec![0.1, 0.2, 0.7, -1.0, 0.0, 1.0]).unwrap();
		let ls = log_softmax(&t);
		let s = softmax(&t);
		for i in 0..2 {
			for j in 0..3 {
				assert!((ls.at(i, j) - s.at(i, j).ln()).abs() < 1e-9, "log_softmax == ln(softmax)");
			}
		}
	}
}
