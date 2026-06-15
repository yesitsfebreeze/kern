use crate::gnn::tensor::Tensor;

pub type PoolFunc = fn(&Tensor) -> Tensor;

pub fn sum_pool(features: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(1, features.cols);
	for i in 0..features.rows {
		for j in 0..features.cols {
			out.data[j] += features.at(i, j);
		}
	}
	out
}

pub fn mean_pool(features: &Tensor) -> Tensor {
	let mut out = sum_pool(features);
	if features.rows == 0 {
		return out; // no rows: mean is the zero vector, not NaN
	}
	let n = features.rows as f64;
	for v in &mut out.data {
		*v /= n;
	}
	out
}

pub fn max_pool(features: &Tensor) -> Tensor {
	let mut out = Tensor::zeros(1, features.cols);
	if features.rows == 0 {
		return out; // no rows: leave zeros rather than NEG_INFINITY sentinels
	}
	for j in 0..features.cols {
		out.data[j] = f64::NEG_INFINITY;
	}
	for i in 0..features.rows {
		for j in 0..features.cols {
			let v = features.at(i, j);
			if v > out.data[j] {
				out.data[j] = v;
			}
		}
	}
	out
}

/// Concatenates sum-pool and mean-pool into a single readout vector.
/// Output width is `2 * features.cols` (sum in the first half, mean in the
/// second); downstream layers must be sized accordingly.
pub fn sum_mean_pool(features: &Tensor) -> Tensor {
	let s = sum_pool(features);
	let m = mean_pool(features);
	let mut data = vec![0.0; 2 * features.cols];
	data[..features.cols].copy_from_slice(&s.data);
	data[features.cols..].copy_from_slice(&m.data);
	Tensor {
		data,
		rows: 1,
		cols: 2 * features.cols,
	}
}

pub struct ReadoutLayer {
	pub pool: PoolFunc,
	pub linear: Option<crate::gnn::layer::LinearLayer>,
}

impl ReadoutLayer {
	pub fn new(pool: PoolFunc, in_features: usize, out_features: usize) -> Self {
		let linear = if out_features > 0 {
			Some(crate::gnn::layer::LinearLayer::new(
				in_features,
				out_features,
			))
		} else {
			None
		};
		Self { pool, linear }
	}

	pub fn forward(&mut self, input: &Tensor) -> Tensor {
		let mut pooled = (self.pool)(input);
		if let Some(ref mut l) = self.linear {
			use crate::gnn::layer::Layer;
			pooled = l.forward(&pooled);
		}
		pooled
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// rows = 3, cols = 2
	fn sample() -> Tensor {
		Tensor::new(3, 2, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap()
	}

	#[test]
	fn sum_pool_adds_columns() {
		let out = sum_pool(&sample());
		assert_eq!(out.rows, 1);
		assert_eq!(out.data, vec![9.0, 12.0]); // 1+3+5, 2+4+6
	}

	#[test]
	fn mean_pool_averages_columns() {
		let out = mean_pool(&sample());
		assert_eq!(out.data, vec![3.0, 4.0]);
	}

	#[test]
	fn max_pool_takes_column_maxima() {
		let out = max_pool(&sample());
		assert_eq!(out.data, vec![5.0, 6.0]);
	}

	#[test]
	fn single_row_pools_to_itself() {
		let t = Tensor::new(1, 2, vec![7.0, -1.0]).unwrap();
		assert_eq!(sum_pool(&t).data, vec![7.0, -1.0]);
		assert_eq!(mean_pool(&t).data, vec![7.0, -1.0]);
		assert_eq!(max_pool(&t).data, vec![7.0, -1.0]);
	}

	#[test]
	fn zero_rows_yield_finite_zeros() {
		let empty = Tensor::zeros(0, 2);
		assert_eq!(mean_pool(&empty).data, vec![0.0, 0.0]);
		let m = max_pool(&empty);
		assert!(
			m.data.iter().all(|v| v.is_finite()),
			"max_pool must not leak NEG_INFINITY on empty input"
		);
		assert_eq!(m.data, vec![0.0, 0.0]);
	}

	#[test]
	fn sum_mean_pool_doubles_width_and_concatenates() {
		let out = sum_mean_pool(&sample());
		assert_eq!(out.cols, 4);
		assert_eq!(out.data, vec![9.0, 12.0, 3.0, 4.0]);
	}
}
