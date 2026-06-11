//! Shared graph health aggregation.
//!
//! Both the REPL `health` command and the MCP `health` tool/resource need the
//! same roll-up over the loaded graph (kern/entity/reason counts, unnamed-kern
//! count, root anchor names). Keeping the loop in one place stops the two
//! surfaces from drifting; each caller layers its own extras on top (the REPL
//! adds queue depth, the MCP surface adds descriptor count).

use crate::base::graph::GraphGnn;

/// Roll-up of the loaded graph's headline counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthStats {
	pub kerns: usize,
	pub entities: usize,
	pub reasons: usize,
	pub unnamed: usize,
	pub anchors: Vec<String>,
}

/// Aggregate headline health counts over every loaded kern. Read-only.
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
		// Anchor names are exactly the loaded root anchors — never more.
		assert!(h.anchors.len() <= h.kerns);
	}
}
