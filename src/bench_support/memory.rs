//! Vector-storage footprint estimate for a built graph: f32 vs int8. PAYLOAD only —
//! excludes HNSW structure, text/metadata, allocator — so it is a lower bound on RSS.

use crate::base::graph::GraphGnn;

#[derive(Debug, Clone)]
pub struct MemoryReport {
	pub entities: usize,
	/// Entities carrying a non-empty embedding.
	pub vectors: usize,
	/// Embedding dimension (the widest seen; 0 when there are no vectors).
	pub dim: usize,
	pub float_vector_bytes: usize,
	pub int8_vector_bytes: usize,
}

impl MemoryReport {
	/// float bytes / int8 bytes; 0 when there are no vectors.
	pub fn quant_ratio(&self) -> f64 {
		if self.int8_vector_bytes == 0 {
			0.0
		} else {
			self.float_vector_bytes as f64 / self.int8_vector_bytes as f64
		}
	}
}

pub fn estimate_memory(g: &GraphGnn) -> MemoryReport {
	let mut entities = 0;
	let mut vectors = 0;
	let mut dim = 0;
	for kern in g.all() {
		for t in kern.entities.values() {
			entities += 1;
			if !t.vector.is_empty() {
				vectors += 1;
				dim = dim.max(t.vector.len());
			}
		}
	}
	MemoryReport {
		entities,
		vectors,
		dim,
		float_vector_bytes: vectors * dim * std::mem::size_of::<f32>(),
		int8_vector_bytes: vectors * dim,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bench_support::build::build_graph;
	use crate::bench_support::trace::{Trace, TraceDoc, TraceQuery};

	fn doc(id: &str) -> TraceDoc {
		TraceDoc {
			id: id.into(),
			text: format!("text for {id} about rust and graphs"),
			kind: None,
		}
	}

	#[test]
	fn estimate_counts_vectors_and_int8_is_4x_smaller() {
		let trace = Trace {
			name: "mem".into(),
			docs: vec![doc("d1"), doc("d2"), doc("d3")],
			queries: vec![TraceQuery {
				id: "q".into(),
				query: "rust".into(),
				expected_ids: vec!["d1".into()],
				mode: "hybrid".into(),
				filter_kind: None,
			}],
		};
		let g = build_graph(&trace);
		let m = estimate_memory(&g);

		assert_eq!(m.entities, 3, "all docs are entities");
		assert_eq!(m.vectors, 3, "all docs are embedded");
		assert!(m.dim > 0, "a real embedding dimension");
		assert_eq!(m.float_vector_bytes, m.vectors * m.dim * 4);
		assert_eq!(m.int8_vector_bytes, m.vectors * m.dim);
		assert!(
			(m.quant_ratio() - 4.0).abs() < 1e-9,
			"int8 is 4x smaller than f32"
		);
	}

	#[test]
	fn empty_graph_reports_zero_and_no_divide_by_zero() {
		let g = build_graph(&Trace {
			name: "e".into(),
			docs: vec![],
			queries: vec![],
		});
		let m = estimate_memory(&g);
		assert_eq!((m.entities, m.vectors, m.dim), (0, 0, 0));
		assert_eq!(m.quant_ratio(), 0.0, "no vectors -> ratio 0, not NaN");
	}
}
