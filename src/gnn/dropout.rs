use crate::gnn::tensor::Tensor;
use rand::RngExt;

pub struct Dropout {
	pub p: f64,
	pub training: bool,
	last_mask: Option<Tensor>,
}

impl Dropout {
	pub fn new(p: f64) -> Self {
		Self {
			p,
			training: true,
			last_mask: None,
		}
	}

	pub fn forward(&mut self, input: &Tensor) -> Tensor {
		if !self.training || self.p == 0.0 {
			self.last_mask = None;
			return input.clone();
		}
		if self.p == 1.0 {
			self.last_mask = Some(Tensor::zeros(input.rows, input.cols));
			return Tensor::zeros(input.rows, input.cols);
		}
		let mut rng = rand::rng();
		// Inverted dropout: survivors scaled by 1/(1-p) at train time, so the
		// eval path needs no rescaling.
		let scale = 1.0 / (1.0 - self.p);
		let mut mask = Tensor::zeros(input.rows, input.cols);
		let mut out = Tensor::zeros(input.rows, input.cols);
		for i in 0..input.data.len() {
			if rng.random::<f64>() >= self.p {
				mask.data[i] = scale;
				out.data[i] = input.data[i] * scale;
			}
		}
		self.last_mask = Some(mask);
		out
	}

	pub fn backward(&self, d_out: &Tensor) -> Tensor {
		match &self.last_mask {
			None => d_out.clone(),
			Some(mask) => d_out.mul(mask).expect("dropout backward mul"),
		}
	}

	pub fn set_training(&mut self, training: bool) {
		self.training = training;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn t() -> Tensor {
		Tensor::new(1, 4, vec![1.0, 2.0, 3.0, 4.0]).unwrap()
	}

	#[test]
	fn p_zero_passes_input_through_unchanged() {
		let mut d = Dropout::new(0.0);
		let out = d.forward(&t());
		assert_eq!(out.data, t().data);
	}

	#[test]
	fn p_one_zeroes_everything() {
		let mut d = Dropout::new(1.0);
		let out = d.forward(&t());
		assert!(out.data.iter().all(|&x| x == 0.0));
	}

	#[test]
	fn eval_mode_bypasses_masking() {
		let mut d = Dropout::new(0.9);
		d.set_training(false);
		let out = d.forward(&t());
		assert_eq!(out.data, t().data, "training=false must not drop");
	}

	#[test]
	fn backward_without_forward_mask_is_identity() {
		let d = Dropout::new(0.5);
		let grad = t();
		assert_eq!(d.backward(&grad).data, grad.data);
	}
}
