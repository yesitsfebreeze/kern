//! Recall digest: a markdown snapshot of the kern's purpose plus its
//! hottest thoughts, written to disk for the Claude-Code SessionStart hook
//! to inject. Pure builder + a thin file writer; no live query path.

use crate::base::graph::GraphGnn;
use crate::base::types::{Entity, EntityStatus};

/// Render the digest markdown: purpose header + up to `k` hottest active
/// thoughts, hottest first.
pub fn build_digest(graph: &GraphGnn, k: usize) -> String {
	let mut out = String::from("# kern memory\n\n");
	let purpose = graph.root.purpose_text.trim();
	if !purpose.is_empty() {
		out.push_str("Purpose: ");
		out.push_str(purpose);
		out.push_str("\n\n");
	}

	let mut ents: Vec<&Entity> = graph
		.kerns
		.values()
		.flat_map(|kern| kern.entities.values())
		.filter(|e| matches!(e.status, EntityStatus::Active))
		.collect();
	ents.sort_by(|a, b| {
		b.heat
			.partial_cmp(&a.heat)
			.unwrap_or(std::cmp::Ordering::Equal)
	});

	out.push_str("## What I know\n\n");
	for e in ents.into_iter().take(k) {
		if let Some(s) = e.statements.first() {
			out.push_str("- ");
			out.push_str(s.trim());
			out.push('\n');
		}
	}
	out
}

/// Render and write the digest to `path`, creating parent dirs. Best effort.
pub fn write_digest(graph: &GraphGnn, path: &std::path::Path, k: usize) {
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	let _ = std::fs::write(path, build_digest(graph, k));
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{
		Acl, ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source,
	};
	use crate::crdt::GCounter;

	fn mk_entity(id: &str, text: &str, heat: f64) -> Entity {
		let mut e = Entity {
			id: id.to_string(),
			root_id: String::new(),
			external_id: String::new(),
			superseded_by: String::new(),
			kind: EntityKind::Claim,
			status: EntityStatus::Active,
			statements: vec![text.to_string()],
			chunks: vec![ChunkPart {
				kind: ChunkPartKind::StatementRef,
				text: String::new(),
				index: 0,
			}],
			vector: vec![0.0; 8],
			gnn_vector: Vec::new(),
			score: 0.0,
			conf_alpha: 2.0,
			conf_beta: 1.0,
			source: Source::Inline {
				hash: id.into(),
				section: String::new(),
			},
			created_at: None,
			acl: Acl::default(),
			access_count: GCounter::new(),
			accessed_at: None,
			heat: heat as f32,
			heat_updated_at: None,
			updated_at: None,
			valid_until: None,
			producer_id: String::new(),
			unlinked_count: 0,
		};
		e.refresh_score();
		e
	}

	#[test]
	fn digest_has_purpose_and_hottest_first_capped() {
		let mut g = GraphGnn::default();
		g.root.purpose_text = "remember durable facts".to_string();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities.insert("a".into(), mk_entity("a", "cold fact", 0.1));
		kern.entities.insert("b".into(), mk_entity("b", "hot fact", 9.0));

		let md = build_digest(&g, 1);
		assert!(md.contains("remember durable facts"), "purpose present");
		assert!(md.contains("hot fact"), "hottest included");
		assert!(!md.contains("cold fact"), "capped at k=1");
	}

	#[test]
	fn empty_graph_yields_header_only() {
		let g = GraphGnn::default();
		let md = build_digest(&g, 10);
		assert!(md.contains("# kern memory"));
	}
}
