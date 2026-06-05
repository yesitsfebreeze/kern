use crate::base::graph::GraphGnn;
use crate::base::math;
use crate::base::reason::add_reason;
use crate::base::types::*;
use crate::crdt::GCounter;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

pub fn find_duplicate(
	graph: &Arc<RwLock<GraphGnn>>,
	vec: &[f64],
	threshold: f64,
) -> Option<String> {
	let g = graph.read().ok()?;
	let hits = g.entity_idx.search(vec, 1, crate::base::constants::DEDUP_EF);
	hits
		.into_iter()
		.find(|h| h.score >= threshold)
		.map(|h| h.id)
}

/// Reinforce an existing near-duplicate entity with a fresh observation.
///
/// CRDT id invariant: an entity id is `content_hash(text)`, and `merge_entity`
/// relies on "equal ids ⇒ identical immutable content" (it joins metadata only,
/// never text). So this MUST NOT overwrite `statements`/`vector` under the
/// existing id — doing so would leave `id = hash(old_text)` while the content is
/// `new_text`, breaking the invariant and causing permanent divergence across
/// federated replicas. A near-dup is corroborating evidence: reinforce
/// confidence. If the new phrasing differs from the stored text, record it as a
/// `Rephrase` edge rather than mutating the canonical text.
pub fn update_existing_entity(
	graph: &Arc<RwLock<GraphGnn>>,
	entity_id: &str,
	new_text: &str,
	new_score: f64,
) {
	let mut g = match graph.write() {
		Ok(g) => g,
		Err(_) => return,
	};
	let kern_id = match g.kern_of_entity(entity_id) {
		Some(kid) => kid.to_string(),
		None => return,
	};
	let kern = match g.get_mut(&kern_id) {
		Some(k) => k,
		None => return,
	};

	let differs = {
		let Some(t) = kern.entities.get_mut(entity_id) else {
			return;
		};
		t.observe_support(new_score);
		t.updated_at = Some(SystemTime::now());
		t.text() != new_text
	};

	if differs {
		let rid = math::reason_id(entity_id, "", ReasonKind::Rephrase, new_text, "");
		let reason = Reason {
			id: rid,
			from: entity_id.to_string(),
			to: String::new(),
			to_kern_id: String::new(),
			to_net_id: String::new(),
			kind: ReasonKind::Rephrase,
			text: new_text.to_string(),
			vector: Vec::new(),
			score: 0.5,
			traversal_count: GCounter::new(),
			producer_id: String::new(),
		};
		add_reason(kern, reason);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::mk_entity;

	fn graph_with_entity(id: &str, text: &str) -> Arc<RwLock<GraphGnn>> {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let e = mk_entity(id, text, 1.0, EntityKind::Claim);
		g.get_mut(&root).unwrap().entities.insert(id.to_string(), e);
		g.index_entity(id, &root);
		Arc::new(RwLock::new(g))
	}

	fn entity(graph: &Arc<RwLock<GraphGnn>>, id: &str) -> Entity {
		let g = graph.read().unwrap();
		let kid = g.kern_of_entity(id).unwrap().to_string();
		g.kerns.get(&kid).unwrap().entities.get(id).unwrap().clone()
	}

	#[test]
	fn same_text_reinforces_confidence_without_rephrase_edge() {
		let graph = graph_with_entity("e1", "the original claim");
		let before = entity(&graph, "e1");

		update_existing_entity(&graph, "e1", "the original claim", 1.0);

		let after = entity(&graph, "e1");
		assert!(after.conf_alpha > before.conf_alpha, "confidence reinforced");
		assert_eq!(after.text(), "the original claim", "text untouched");
		assert!(after.updated_at.is_some(), "updated_at bumped");
		// No Rephrase edge for identical text.
		let g = graph.read().unwrap();
		let kid = g.kern_of_entity("e1").unwrap();
		let any_rephrase = g
			.kerns
			.get(kid)
			.unwrap()
			.reasons
			.values()
			.any(|r| r.kind == ReasonKind::Rephrase);
		assert!(!any_rephrase, "no rephrase edge for exact-same text");
	}

	#[test]
	fn different_text_preserves_id_invariant_and_records_rephrase() {
		// SECURITY/CORRECTNESS regression: a near-dup with DIFFERENT text must
		// NOT mutate the stored text/vector under the content-hash id (that would
		// make id != hash(content) and break CRDT convergence). The alternate
		// phrasing is captured as a Rephrase edge instead.
		let graph = graph_with_entity("e1", "the original claim");
		let before = entity(&graph, "e1");

		update_existing_entity(&graph, "e1", "a reworded version of the claim", 1.0);

		let after = entity(&graph, "e1");
		assert_eq!(after.id, "e1", "id unchanged");
		assert_eq!(after.text(), "the original claim", "stored text NOT overwritten");
		assert_eq!(after.vector, before.vector, "vector NOT overwritten");
		assert!(after.conf_alpha > before.conf_alpha, "confidence reinforced");

		let g = graph.read().unwrap();
		let kid = g.kern_of_entity("e1").unwrap();
		let rephrase: Vec<_> = g
			.kerns
			.get(kid)
			.unwrap()
			.reasons
			.values()
			.filter(|r| r.kind == ReasonKind::Rephrase)
			.collect();
		assert_eq!(rephrase.len(), 1, "exactly one rephrase edge");
		assert_eq!(rephrase[0].from, "e1");
		assert_eq!(rephrase[0].text, "a reworded version of the claim");
	}

	#[test]
	fn rephrase_edge_is_idempotent_under_repeat() {
		// Re-observing the same alternate phrasing must not pile up duplicate
		// edges — reason_id is content-addressed, so add_reason de-dupes.
		let graph = graph_with_entity("e1", "the original claim");
		update_existing_entity(&graph, "e1", "reworded claim", 1.0);
		update_existing_entity(&graph, "e1", "reworded claim", 1.0);

		let g = graph.read().unwrap();
		let kid = g.kern_of_entity("e1").unwrap();
		let count = g
			.kerns
			.get(kid)
			.unwrap()
			.reasons
			.values()
			.filter(|r| r.kind == ReasonKind::Rephrase)
			.count();
		assert_eq!(count, 1, "duplicate rephrase observations collapse to one edge");
	}
}
