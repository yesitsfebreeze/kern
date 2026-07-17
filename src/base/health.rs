use crate::base::graph::GraphGnn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStats {
	pub kerns: usize,
	pub entities: usize,
	pub reasons: usize,
	pub unnamed: usize,
	pub anchors: Vec<String>,
}

pub fn graph_health_stats(g: &GraphGnn) -> HealthStats {
	let kerns = g.all();
	let mut entities = 0usize;
	let mut reasons = 0usize;
	let mut unnamed = 0usize;
	for k in &kerns {
		entities += k.entities.len();
		reasons += k.reasons.len();
		if k.is_unnamed() {
			unnamed += 1;
		}
	}
	let anchors: Vec<String> = crate::base::accept::root_anchor_ids(g)
		.iter()
		.filter_map(|cid| g.loaded(cid))
		.map(|c| c.anchor_text.clone())
		.collect();
	HealthStats {
		kerns: kerns.len(),
		entities,
		reasons,
		unnamed,
		anchors,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn empty_graph_reports_no_entities_or_reasons() {
		let g = GraphGnn::new();
		let h = graph_health_stats(&g);
		assert_eq!(h.entities, 0, "fresh graph has no entities");
		assert_eq!(h.reasons, 0, "fresh graph has no reasons");
		assert!(h.kerns >= 1, "at least the root kern is present");
		assert!(h.anchors.len() <= h.kerns);
	}
}
