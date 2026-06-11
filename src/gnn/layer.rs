

use crate::gnn::tensor::Tensor;
use crate::gnn::GnnError;

pub trait Layer {
	fn forward(&mut self, input: &Tensor) -> Tensor;
	fn parameters(&self) -> Vec<&Tensor>;
	fn parameters_mut(&mut self) -> Vec<&mut Tensor>;
}

pub trait Backward {
	fn backward(&mut self, d_out: &Tensor) -> Tensor;
	fn param_grads(&self) -> Vec<&Tensor>;
	fn param_grads_mut(&mut self) -> Vec<&mut Tensor>;
	fn zero_grads(&mut self);
}

pub struct LinearLayer {
	pub weight: Tensor, // (in_features, out_features)
	pub bias: Tensor,   // (1, out_features)
	last_input: Option<Tensor>,
	d_weight: Tensor,
	d_bias: Tensor,
}

impl LinearLayer {
	pub fn new(in_features: usize, out_features: usize) -> Self {
		let mut rng = rand::rng();
		Self::with_rng(in_features, out_features, &mut rng)
	}

	/// Construct a `LinearLayer` with deterministic weight init from a
	/// seeded RNG. Bias is zero-initialized (no RNG draw). Use this
	/// constructor in tests so loss-decrease assertions are reproducible.
	pub fn with_rng<R: rand::Rng>(
		in_features: usize,
		out_features: usize,
		rng: &mut R,
	) -> Self {
		let scale = (2.0 / (in_features + out_features) as f64).sqrt();
		let weight = Tensor::rand_with(in_features, out_features, scale, rng);
		let bias = Tensor::zeros(1, out_features);
		let d_weight = Tensor::zeros(in_features, out_features);
		let d_bias = Tensor::zeros(1, out_features);
		Self {
			weight,
			bias,
			last_input: None,
			d_weight,
			d_bias,
		}
	}

	/// Fallible backward pass. Returns [`GnnError::MissingForwardState`] when
	/// invoked before a successful `forward` (or after reset) instead of
	/// panicking, and bubbles tensor shape errors as [`GnnError::Tensor`].
	/// Mirrors [`GATLayer::try_backward_graph`](crate::gnn::gat::GATLayer); the
	/// infallible [`Backward::backward`] delegates here.
	pub fn try_backward(&mut self, d_out: &Tensor) -> Result<Tensor, GnnError> {
		let input = self
			.last_input
			.as_ref()
			.ok_or(GnnError::MissingForwardState("linear::last_input"))?;
		let dw = input.transpose().matmul(d_out)?;
		self.d_weight.add_inplace(&dw)?;
		for i in 0..d_out.rows {
			for j in 0..d_out.cols {
				self.d_bias.data[j] += d_out.at(i, j);
			}
		}
		Ok(d_out.matmul(&self.weight.transpose())?)
	}
}

impl Layer for LinearLayer {
	fn forward(&mut self, input: &Tensor) -> Tensor {
		self.last_input = Some(input.clone());
		let out = input.matmul(&self.weight).expect("linear forward matmul");
		out.add_row_vec(&self.bias).expect("linear forward bias")
	}

	fn parameters(&self) -> Vec<&Tensor> {
		vec![&self.weight, &self.bias]
	}

	fn parameters_mut(&mut self) -> Vec<&mut Tensor> {
		vec![&mut self.weight, &mut self.bias]
	}
}

impl Backward for LinearLayer {
	fn backward(&mut self, d_out: &Tensor) -> Tensor {
		match self.try_backward(d_out) {
			Ok(t) => t,
			Err(e) => {
				tracing::error!(error = %e, "LinearLayer backward failed; returning zero gradient");
				// dInput is (n_samples, in_features); in_features == weight.rows.
				Tensor::zeros(d_out.rows, self.weight.rows)
			}
		}
	}

	fn param_grads(&self) -> Vec<&Tensor> {
		vec![&self.d_weight, &self.d_bias]
	}

	fn param_grads_mut(&mut self) -> Vec<&mut Tensor> {
		vec![&mut self.d_weight, &mut self.d_bias]
	}

	fn zero_grads(&mut self) {
		// In place — keeps the allocations, just zeroes them.
		self.d_weight.fill(0.0);
		self.d_bias.fill(0.0);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rand::rngs::StdRng;
	use rand::SeedableRng;

	fn layer(in_f: usize, out_f: usize) -> LinearLayer {
		let mut rng = StdRng::seed_from_u64(7);
		LinearLayer::with_rng(in_f, out_f, &mut rng)
	}

	#[test]
	fn forward_projects_to_out_features_width() {
		let mut l = layer(4, 3);
		let y = l.forward(&Tensor::zeros(2, 4)); // 2 samples x 4 features
		assert_eq!((y.rows, y.cols), (2, 3), "n_samples x out_features");
	}

	#[test]
	fn backward_dinput_shape_and_grad_accumulation() {
		let mut l = layer(4, 3);
		let x = Tensor::new(2, 4, vec![1.0; 8]).unwrap();
		let _ = l.forward(&x);
		let d_out = Tensor::new(2, 3, vec![1.0; 6]).unwrap();
		let d_in = l.backward(&d_out);

		assert_eq!((d_in.rows, d_in.cols), (2, 4), "dInput matches input shape");
		// d_bias[j] = sum over rows of d_out[:,j] = 2 rows * 1.0 = 2.0.
		assert!(l.d_bias.data.iter().all(|&b| (b - 2.0).abs() < 1e-12), "d_bias = column sums of d_out");
		// d_weight = Xᵀ·dOut; X all 1s (2x4), dOut all 1s (2x3) -> each elem = 2.0.
		assert!(l.d_weight.data.iter().all(|&w| (w - 2.0).abs() < 1e-12), "d_weight = Xᵀ·dOut");
	}

	#[test]
	fn backward_accumulates_across_calls_until_zeroed() {
		let mut l = layer(2, 2);
		let x = Tensor::new(1, 2, vec![1.0, 1.0]).unwrap();
		let d_out = Tensor::new(1, 2, vec![1.0, 1.0]).unwrap();
		let _ = l.forward(&x);
		l.backward(&d_out);
		l.backward(&d_out);
		assert!(l.d_bias.data.iter().all(|&b| (b - 2.0).abs() < 1e-12), "two calls accumulate d_bias");

		l.zero_grads();
		assert!(l.d_weight.data.iter().all(|&w| w == 0.0), "zero_grads clears d_weight in place");
		assert!(l.d_bias.data.iter().all(|&b| b == 0.0));
	}

	#[test]
	fn try_backward_before_forward_is_a_missing_state_error() {
		let mut l = layer(2, 2);
		let d_out = Tensor::new(1, 2, vec![1.0, 1.0]).unwrap();
		assert!(matches!(l.try_backward(&d_out).unwrap_err(), GnnError::MissingForwardState(_)));

		// The infallible trait method degrades to a zero gradient of input shape.
		let z = l.backward(&d_out);
		assert_eq!((z.rows, z.cols), (1, 2), "fallback dInput is (n_samples, in_features)");
		assert!(z.data.iter().all(|&v| v == 0.0));
	}
}
