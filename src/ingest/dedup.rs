use crate::base::accept::merge_duplicate;
use crate::base::graph::GraphGnn;
use crate::base::types::*;
use std::sync::Arc;

use parking_lot::RwLock;

pub fn find_duplicate(
	graph: &Arc<RwLock<GraphGnn>>,
	vec: &[f32],
	threshold: f64,
) -> Option<String> {
	let g = graph.read();
	let hits = g
		.entity_idx
		.search(vec, 1, crate::base::constants::DEDUP_EF);
	hits
		.into_iter()
		.find(|h| h.score >= threshold)
		.map(|h| h.id)
}

// `incoming_acl` is the ingesting caller's ACL — see `merge_duplicate` for why a
// dedup must not carry it across a boundary.
#[allow(clippy::too_many_arguments)]
pub fn update_existing_entity(
	graph: &Arc<RwLock<GraphGnn>>,
	entity_id: &str,
	new_text: &str,
	new_score: f64,
	incoming_kind: EntityKind,
	incoming_valid_until: Option<std::time::SystemTime>,
	incoming_acl: &Acl,
	on_supersede_candidate: Option<&crate::ingest::worker::DeferContradictionFn>,
) {
	let outcome = merge_duplicate(
		&mut graph.write(),
		entity_id,
		new_text,
		new_score,
		incoming_kind,
		incoming_valid_until,
		incoming_acl,
	);

	// Only a SAME-KIND near-dup may supersede (a preference must not supersede a fact).
	if let Some(o) = outcome {
		if let (Some(rid), true, Some(hook)) = (o.rephrase_id, o.same_kind, on_supersede_candidate) {
			hook(&o.kern_id, &rid);
		}
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
		let g = graph.read();
		let kid = g.kern_of_entity(id).unwrap().to_string();
		g.kerns.get(&kid).unwrap().entities.get(id).unwrap().clone()
	}

	fn graph_with_vec_entity(id: &str, vec: Vec<f32>) -> Arc<RwLock<GraphGnn>> {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let mut e = mk_entity(id, "text", 1.0, EntityKind::Claim);
		e.vector = vec;
		g.get_mut(&root).unwrap().entities.insert(id.to_string(), e);
		g.rebuild_index();
		Arc::new(RwLock::new(g))
	}

	#[test]
	fn find_duplicate_matches_above_threshold_and_skips_below() {
		let graph = graph_with_vec_entity("e1", vec![1.0, 0.0, 0.0]);
		assert_eq!(
			find_duplicate(&graph, &[1.0, 0.0, 0.0], 0.9).as_deref(),
			Some("e1")
		);
		assert_eq!(find_duplicate(&graph, &[0.0, 1.0, 0.0], 0.9), None);
		assert_eq!(find_duplicate(&graph, &[0.9, 0.1, 0.0], 0.999), None);
		assert_eq!(
			find_duplicate(&graph, &[0.9, 0.1, 0.0], 0.9).as_deref(),
			Some("e1")
		);
	}

	#[test]
	fn find_duplicate_on_empty_graph_is_none() {
		let graph = Arc::new(RwLock::new(GraphGnn::new()));
		assert_eq!(find_duplicate(&graph, &[1.0, 0.0], 0.5), None);
	}

	#[test]
	fn same_text_reinforces_confidence_without_rephrase_edge() {
		let graph = graph_with_entity("e1", "the original claim");
		let before = entity(&graph, "e1");

		update_existing_entity(
			&graph,
			"e1",
			"the original claim",
			1.0,
			EntityKind::Claim,
			None,
			&Acl::default(),
			None,
		);

		let after = entity(&graph, "e1");
		assert!(
			after.conf_alpha > before.conf_alpha,
			"confidence reinforced"
		);
		assert_eq!(after.text(), "the original claim", "text untouched");
		assert!(after.updated_at.is_some(), "updated_at bumped");
		let g = graph.read();
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
		let graph = graph_with_entity("e1", "the original claim");
		let before = entity(&graph, "e1");

		update_existing_entity(
			&graph,
			"e1",
			"a reworded version of the claim",
			1.0,
			EntityKind::Claim,
			None,
			&Acl::default(),
			None,
		);

		let after = entity(&graph, "e1");
		assert_eq!(after.id, "e1", "id unchanged");
		assert_eq!(
			after.text(),
			"the original claim",
			"stored text NOT overwritten"
		);
		assert_eq!(after.vector, before.vector, "vector NOT overwritten");
		assert!(
			after.conf_alpha > before.conf_alpha,
			"confidence reinforced"
		);

		let g = graph.read();
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
		let graph = graph_with_entity("e1", "the original claim");
		update_existing_entity(
			&graph,
			"e1",
			"reworded claim",
			1.0,
			EntityKind::Claim,
			None,
			&Acl::default(),
			None,
		);
		update_existing_entity(
			&graph,
			"e1",
			"reworded claim",
			1.0,
			EntityKind::Claim,
			None,
			&Acl::default(),
			None,
		);

		let g = graph.read();
		let kid = g.kern_of_entity("e1").unwrap();
		let count = g
			.kerns
			.get(kid)
			.unwrap()
			.reasons
			.values()
			.filter(|r| r.kind == ReasonKind::Rephrase)
			.count();
		assert_eq!(
			count, 1,
			"duplicate rephrase observations collapse to one edge"
		);
	}

	fn rephrase_texts(graph: &Arc<RwLock<GraphGnn>>, id: &str) -> Vec<String> {
		let g = graph.read();
		let kid = g.kern_of_entity(id).unwrap().to_string();
		g.kerns
			.get(&kid)
			.unwrap()
			.reasons
			.values()
			.filter(|r| r.kind == ReasonKind::Rephrase)
			.map(|r| r.text.clone())
			.collect()
	}

	// The dedup ACL rule. A survivor keeps its own ACL — an id IS its content
	// hash, so one text cannot exist under two audiences. That makes the Rephrase
	// edge the leak: it stores the incoming text verbatim, a Reason carries no ACL
	// of its own, and every read surface that renders an entity renders its edges.
	// A scoped ingest landing within dedup range of a PUBLIC thought would have
	// published its text to everyone.
	#[test]
	fn a_scoped_dedup_does_not_write_its_text_onto_a_public_survivor() {
		let graph = graph_with_entity("e1", "the original claim");
		let before = entity(&graph, "e1");
		let scoped = Acl {
			scope: "acme".into(),
			..Default::default()
		};

		update_existing_entity(
			&graph,
			"e1",
			"the vault code is 4815162342",
			1.0,
			EntityKind::Claim,
			None,
			&scoped,
			None,
		);

		assert!(
			rephrase_texts(&graph, "e1").is_empty(),
			"a public survivor must not carry a scoped ingest's wording: {:?}",
			rephrase_texts(&graph, "e1")
		);
		// Corroboration is metadata about a statement, not the statement — it still merges.
		let after = entity(&graph, "e1");
		assert!(
			after.conf_alpha > before.conf_alpha,
			"support still observed across the boundary"
		);
		assert_eq!(after.acl, Acl::default(), "the survivor keeps its own ACL");
	}

	// Same rule in the other direction, and the reason it is `!=` rather than a
	// one-way check: a public ingest must not slip its wording onto a scoped
	// entity either, where a member would read it as scoped content.
	#[test]
	fn a_public_dedup_does_not_write_its_text_onto_a_scoped_survivor() {
		let graph = graph_with_entity("e1", "the original claim");
		{
			let mut g = graph.write();
			let kid = g.kern_of_entity("e1").unwrap().to_string();
			g.get_mut(&kid).unwrap().entities.get_mut("e1").unwrap().acl = Acl {
				scope: "acme".into(),
				..Default::default()
			};
		}

		update_existing_entity(
			&graph,
			"e1",
			"a reworded version of the claim",
			1.0,
			EntityKind::Claim,
			None,
			&Acl::default(),
			None,
		);

		assert!(
			rephrase_texts(&graph, "e1").is_empty(),
			"the wording does not cross the boundary in either direction"
		);
	}

	// And the boundary is the ACL, not the mere presence of one: same ACL still
	// rephrases, or the guard would have quietly disabled dedup for every scoped
	// corpus.
	#[test]
	fn a_matching_acl_still_records_the_rephrase() {
		let graph = graph_with_entity("e1", "the original claim");
		let scoped = Acl {
			scope: "acme".into(),
			users: vec!["alice".into()],
			groups: Vec::new(),
		};
		{
			let mut g = graph.write();
			let kid = g.kern_of_entity("e1").unwrap().to_string();
			g.get_mut(&kid).unwrap().entities.get_mut("e1").unwrap().acl = scoped.clone();
		}

		update_existing_entity(
			&graph,
			"e1",
			"a reworded version of the claim",
			1.0,
			EntityKind::Claim,
			None,
			&scoped,
			None,
		);

		assert_eq!(
			rephrase_texts(&graph, "e1"),
			vec!["a reworded version of the claim".to_string()],
			"inside one audience the rephrase edge works exactly as before"
		);
	}
}
