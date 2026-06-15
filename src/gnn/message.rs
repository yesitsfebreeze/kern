use crate::gnn::tensor::Tensor;

/// Reduction over a node's incoming messages — e.g. [`sum_aggregate`],
/// [`mean_aggregate`], [`max_aggregate`].
///
/// A bare `fn` pointer (rather than `Box<dyn Fn>`) is deliberate: every
/// aggregator here is a pure, stateless, element-wise reduction, so the pointer
/// is `Copy`, allocation-free, and trivially shared across layers and threads.
/// If a *stateful* aggregator is ever needed (learnable or attention-weighted
/// pooling), model it as its own [`crate::gnn::backward::GraphLayer`] (see
/// `GATLayer`) instead of widening this alias to a trait object — that keeps the
/// hot reduction path monomorphic and the simple aggregators cheap.
pub type AggregateFunc = fn(&[Tensor]) -> Option<Tensor>;

pub fn sum_aggregate(messages: &[Tensor]) -> Option<Tensor> {
	if messages.is_empty() {
		return None;
	}
	let mut result = messages[0].clone();
	for m in &messages[1..] {
		for (a, b) in result.data.iter_mut().zip(&m.data) {
			*a += *b;
		}
	}
	Some(result)
}

pub fn mean_aggregate(messages: &[Tensor]) -> Option<Tensor> {
	let mut result = sum_aggregate(messages)?;
	let n = messages.len() as f64;
	for v in &mut result.data {
		*v /= n;
	}
	Some(result)
}

pub fn max_aggregate(messages: &[Tensor]) -> Option<Tensor> {
	if messages.is_empty() {
		return None;
	}
	let mut result = messages[0].clone();
	for m in &messages[1..] {
		for (a, b) in result.data.iter_mut().zip(&m.data) {
			*a = a.max(*b);
		}
	}
	Some(result)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn row(data: Vec<f64>) -> Tensor {
		let cols = data.len();
		Tensor::new(1, cols, data).unwrap()
	}

	#[test]
	fn empty_messages_yield_none() {
		assert!(sum_aggregate(&[]).is_none());
		assert!(mean_aggregate(&[]).is_none());
		assert!(max_aggregate(&[]).is_none());
	}

	#[test]
	fn single_message_is_identity() {
		let m = vec![row(vec![1.0, -2.0, 3.0])];
		assert_eq!(sum_aggregate(&m).unwrap().data, vec![1.0, -2.0, 3.0]);
		assert_eq!(mean_aggregate(&m).unwrap().data, vec![1.0, -2.0, 3.0]);
		assert_eq!(max_aggregate(&m).unwrap().data, vec![1.0, -2.0, 3.0]);
	}

	#[test]
	fn reductions_are_elementwise() {
		let m = vec![
			row(vec![1.0, 5.0]),
			row(vec![3.0, 2.0]),
			row(vec![-1.0, 4.0]),
		];
		assert_eq!(sum_aggregate(&m).unwrap().data, vec![3.0, 11.0]);
		assert_eq!(mean_aggregate(&m).unwrap().data, vec![1.0, 11.0 / 3.0]);
		assert_eq!(max_aggregate(&m).unwrap().data, vec![3.0, 5.0]);
	}
}
