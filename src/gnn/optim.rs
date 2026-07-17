use crate::gnn::tensor::Tensor;

pub trait Optimizer {
	fn step(&mut self, params: &mut [&mut Tensor], grads: &[&Tensor]);
	/// Zeroes the gradient tensors PASSED IN — never model-owned grads nor the
	/// optimizer's own momentum / Adam moment state.
	fn zero_grad(&self, grads: &mut [Tensor]) {
		for g in grads.iter_mut() {
			for v in &mut g.data {
				*v = 0.0;
			}
		}
	}
}

pub struct SGD {
	pub lr: f64,
	pub momentum: f64,
	velocity: Vec<Tensor>,
}

impl SGD {
	pub fn new(lr: f64) -> Self {
		Self {
			lr,
			momentum: 0.0,
			velocity: Vec::new(),
		}
	}

	pub fn with_momentum(lr: f64, momentum: f64) -> Self {
		Self {
			lr,
			momentum,
			velocity: Vec::new(),
		}
	}
}

impl Optimizer for SGD {
	fn step(&mut self, params: &mut [&mut Tensor], grads: &[&Tensor]) {
		if self.momentum > 0.0 && self.velocity.is_empty() {
			self.velocity = params
				.iter()
				.map(|p| Tensor::zeros(p.rows, p.cols))
				.collect();
		}

		for (i, (param, grad)) in params.iter_mut().zip(grads.iter()).enumerate() {
			if self.momentum > 0.0 {
				let (momentum, lr) = (self.momentum, self.lr);
				let v = &mut self.velocity[i];
				for (j, pj) in param.data.iter_mut().enumerate() {
					v.data[j] = momentum * v.data[j] + grad.data[j];
					*pj -= lr * v.data[j];
				}
			} else {
				for (pj, gj) in param.data.iter_mut().zip(&grad.data) {
					*pj -= self.lr * gj;
				}
			}
		}
	}
}

pub struct Adam {
	pub lr: f64,
	pub beta1: f64,
	pub beta2: f64,
	pub epsilon: f64,
	step_count: usize,
	m: Vec<Tensor>,
	v: Vec<Tensor>,
}

impl Adam {
	pub fn new(lr: f64) -> Self {
		Self {
			lr,
			beta1: 0.9,
			beta2: 0.999,
			epsilon: 1e-8,
			step_count: 0,
			m: Vec::new(),
			v: Vec::new(),
		}
	}
}

impl Optimizer for Adam {
	fn step(&mut self, params: &mut [&mut Tensor], grads: &[&Tensor]) {
		if self.m.is_empty() {
			self.m = params
				.iter()
				.map(|p| Tensor::zeros(p.rows, p.cols))
				.collect();
			self.v = params
				.iter()
				.map(|p| Tensor::zeros(p.rows, p.cols))
				.collect();
		}
		self.step_count += 1;
		let t = self.step_count as f64;
		let bias_c1 = 1.0 - self.beta1.powf(t);
		let bias_c2 = 1.0 - self.beta2.powf(t);

		for (i, (param, grad)) in params.iter_mut().zip(grads.iter()).enumerate() {
			for j in 0..param.data.len() {
				let g = grad.data[j];
				self.m[i].data[j] = self.beta1 * self.m[i].data[j] + (1.0 - self.beta1) * g;
				self.v[i].data[j] = self.beta2 * self.v[i].data[j] + (1.0 - self.beta2) * g * g;

				let m_hat = self.m[i].data[j] / bias_c1;
				let v_hat = self.v[i].data[j] / bias_c2;

				param.data[j] -= self.lr * m_hat / (v_hat.sqrt() + self.epsilon);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scalar(v: f64) -> Tensor {
		Tensor::new(1, 1, vec![v]).unwrap()
	}

	#[test]
	fn sgd_plain_step() {
		let mut p = scalar(1.0);
		let g = scalar(0.5);
		let mut opt = SGD::new(0.1);
		opt.step(&mut [&mut p], &[&g]);
		assert!((p.data[0] - 0.95).abs() < 1e-12); // 1.0 - 0.1*0.5
	}

	#[test]
	fn sgd_momentum_accumulates() {
		let mut p = scalar(0.0);
		let g = scalar(1.0);
		let mut opt = SGD::with_momentum(0.1, 0.9);
		opt.step(&mut [&mut p], &[&g]); // v=1.0   -> p -= 0.1   => -0.10
		opt.step(&mut [&mut p], &[&g]); // v=1.9   -> p -= 0.19  => -0.29
		assert!((p.data[0] - (-0.29)).abs() < 1e-12);
	}

	#[test]
	fn adam_first_step_is_lr_scaled_sign() {
		// At t=1 the bias-corrected update is lr * g / (|g| + eps) ~= lr*sign(g).
		let mut p = scalar(0.0);
		let g = scalar(2.0);
		let mut opt = Adam::new(0.1);
		opt.step(&mut [&mut p], &[&g]);
		assert!((p.data[0] - (-0.1)).abs() < 1e-6, "got {}", p.data[0]);
	}

	#[test]
	fn adam_keeps_independent_moment_state_per_parameter() {
		let mut p0 = scalar(0.0);
		let mut p1 = scalar(0.0);
		let g0 = scalar(2.0);
		let g1 = scalar(-2.0);
		let mut opt = Adam::new(0.1);
		opt.step(&mut [&mut p0, &mut p1], &[&g0, &g1]);
		assert!((p0.data[0] - (-0.1)).abs() < 1e-6, "p0 {}", p0.data[0]);
		assert!((p1.data[0] - 0.1).abs() < 1e-6, "p1 {}", p1.data[0]);
	}

	#[test]
	fn sgd_momentum_velocity_is_independent_per_parameter() {
		let mut p0 = scalar(0.0);
		let mut p1 = scalar(0.0);
		let g0 = scalar(1.0);
		let g1 = scalar(-1.0);
		let mut opt = SGD::with_momentum(0.1, 0.9);
		opt.step(&mut [&mut p0, &mut p1], &[&g0, &g1]); // v0=1, v1=-1
		opt.step(&mut [&mut p0, &mut p1], &[&g0, &g1]); // v0=1.9, v1=-1.9
		assert!((p0.data[0] - (-0.29)).abs() < 1e-12, "p0 {}", p0.data[0]);
		assert!((p1.data[0] - 0.29).abs() < 1e-12, "p1 {}", p1.data[0]);
	}
}
