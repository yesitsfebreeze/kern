use super::graph::GraphGnn;
use super::types::{Kern, Reason, ReasonKind};
use std::collections::HashSet;

/// All reason ids incident to `entity_id` — outgoing (`by_from`) then incoming
/// (`by_to`). Single edge-collection source for retrieval, MCP, and the CLI.
pub(crate) fn collect_reason_ids(kern: &Kern, entity_id: &str) -> Vec<String> {
	let mut ids = Vec::new();
	if let Some(from_ids) = kern.by_from.get(entity_id) {
		ids.extend(from_ids.iter().cloned());
	}
	if let Some(to_ids) = kern.by_to.get(entity_id) {
		ids.extend(to_ids.iter().cloned());
	}
	ids
}

/// Every superseded ancestor of an active head: `Supersedes` edges point new -> old,
/// so the walk follows outgoing edges. `seen` terminates cyclic chains.
pub fn superseded_ancestors(g: &GraphGnn, entity_id: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut seen: HashSet<String> = HashSet::new();
	let mut frontier = vec![entity_id.to_string()];
	while let Some(cur) = frontier.pop() {
		let Some(kid) = g.kern_of_entity(&cur).map(str::to_string) else {
			continue;
		};
		let Some(kern) = g.loaded(&kid) else {
			continue;
		};
		let Some(edges) = kern.by_from.get(&cur) else {
			continue;
		};
		for rid in edges {
			if let Some(r) = kern.reasons.get(rid) {
				if r.kind == ReasonKind::Supersedes && !r.to.is_empty() && seen.insert(r.to.clone()) {
					out.push(r.to.clone());
					frontier.push(r.to.clone());
				}
			}
		}
	}
	out
}

pub fn add_reason(kern: &mut Kern, reason: Reason) {
	let id = reason.id.clone();
	let from = reason.from.clone();
	let to = reason.to.clone();
	// Index adjacency only for NEW ids: `by_from`/`by_to` are Vecs, so re-adding
	// the same edge id would append a duplicate and leave a stale entry on remove.
	let is_new = kern.reasons.insert(id.clone(), reason).is_none();
	if !is_new {
		return;
	}
	kern.by_from.entry(from).or_default().push(id.clone());
	if !to.is_empty() {
		kern.by_to.entry(to).or_default().push(id);
	}
}

pub fn remove_reason(kern: &mut Kern, id: &str) {
	let reason = match kern.reasons.remove(id) {
		Some(r) => r,
		None => return,
	};
	remove_string_from_vec(kern.by_from.get_mut(&reason.from), id);
	if !reason.to.is_empty() {
		remove_string_from_vec(kern.by_to.get_mut(&reason.to), id);
	}
}

/// Relocate an entity and its OUTGOING reasons (a kern hosts a reason iff it
/// hosts its `from`); incoming stay put. `to_net_id` set ⇒ never stamp over it.
pub fn move_entity(g: &mut GraphGnn, from_kern_id: &str, to_kern_id: &str, entity_id: &str) {
	if from_kern_id == to_kern_id {
		return;
	}

	let src = match g.kerns.get_mut(from_kern_id) {
		Some(k) => k,
		None => return,
	};

	let entity = match src.entities.remove(entity_id) {
		Some(t) => t,
		None => return,
	};

	let (outgoing_rids, incoming_rids) = reasons_touching(src, entity_id);

	for rid in &incoming_rids {
		if let Some(reason) = src.reasons.get_mut(rid) {
			if reason.to_kern_id.is_empty() && reason.to_net_id.is_empty() {
				reason.to_kern_id = to_kern_id.to_string();
			}
		}
	}

	let mut moved_reasons = Vec::with_capacity(outgoing_rids.len());
	for rid in &outgoing_rids {
		if let Some(reason) = src.reasons.remove(rid) {
			remove_string_from_vec(src.by_from.get_mut(&reason.from), rid);
			if !reason.to.is_empty() {
				remove_string_from_vec(src.by_to.get_mut(&reason.to), rid);
			}
			moved_reasons.push(reason);
		}
	}

	let dst = match g.kerns.get_mut(to_kern_id) {
		Some(k) => k,
		None => return,
	};

	let moved_ids: Vec<String> = moved_reasons.iter().map(|r| r.id.clone()).collect();
	for mut reason in moved_reasons {
		if !reason.to.is_empty()
			&& reason.to != entity_id
			&& reason.to_kern_id.is_empty()
			&& reason.to_net_id.is_empty()
		{
			reason.to_kern_id = from_kern_id.to_string();
		}
		add_reason(dst, reason);
	}
	dst.entities.insert(entity_id.to_string(), entity);

	g.index_entity(entity_id, to_kern_id);
	for rid in &moved_ids {
		g.index_reason(rid, to_kern_id);
	}
}

/// Remove an entity and cascade through every index that referenced it. Active
/// Facts are immune (Superseded facts are not); anything missing is a silent no-op.
pub fn remove_entity(g: &mut GraphGnn, kern_id: &str, id: &str) {
	let kern = match g.kerns.get_mut(kern_id) {
		Some(k) => k,
		None => return,
	};

	if let Some(t) = kern.entities.get(id) {
		// A SUPERSEDED fact is invalidated history, not durable knowledge — the
		// bi-temporal GC spills it to the cold tier and drops it here.
		if t.is_fact() && !t.is_superseded() {
			return;
		}
	}
	if kern.entities.remove(id).is_none() {
		return;
	}

	let (outgoing, incoming) = reasons_touching(kern, id);
	let rids: Vec<String> = outgoing.into_iter().chain(incoming).collect();
	for rid in &rids {
		remove_reason(kern, rid);
	}
	kern.by_from.remove(id);
	kern.by_to.remove(id);

	for rid in &rids {
		g.reason_idx.delete(rid);
		g.unindex_reason(rid);
	}

	g.entity_idx.delete(id);
	g.gnn_entity_idx.delete(id);
	g.unindex_entity(id);

	if let Some(lex) = g.lexical() {
		lex.remove(id);
	}
}

/// Partition the reasons touching `entity_id` into `(outgoing, incoming)` rid
/// clones; a self-loop counts once, as outgoing.
fn reasons_touching(kern: &Kern, entity_id: &str) -> (Vec<String>, Vec<String>) {
	let outgoing: Vec<String> = kern.by_from.get(entity_id).cloned().unwrap_or_default();
	let mut incoming = Vec::new();
	if let Some(to_rids) = kern.by_to.get(entity_id) {
		for rid in to_rids {
			if !outgoing.contains(rid) {
				incoming.push(rid.clone());
			}
		}
	}
	(outgoing, incoming)
}

/// Remove the first occurrence of `s`. Linear scan intentional: adjacency lists
/// stay tens of ids, and swapping the serde-persisted `Vec` is a format change.
fn remove_string_from_vec(vec: Option<&mut Vec<String>>, s: &str) {
	if let Some(v) = vec {
		if let Some(pos) = v.iter().position(|x| x == s) {
			v.remove(pos);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, EntityKind, Kern};

	use crate::test_support::{edge, entity_vec as ent};

	#[test]
	fn superseded_ancestors_walks_the_supersedes_chain_backward() {
		// Chain: newest -> mid -> old (each Supersedes edge points new->old).
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		for id in ["newest", "mid", "old"] {
			g.get_mut(&root).unwrap().entities.insert(
				id.into(),
				Entity {
					id: id.into(),
					..Default::default()
				},
			);
			g.index_entity(id, &root);
		}
		let k = g.get_mut(&root).unwrap();
		add_reason(
			k,
			Reason {
				id: "s1".into(),
				from: "newest".into(),
				to: "mid".into(),
				kind: ReasonKind::Supersedes,
				..Default::default()
			},
		);
		add_reason(
			k,
			Reason {
				id: "s2".into(),
				from: "mid".into(),
				to: "old".into(),
				kind: ReasonKind::Supersedes,
				..Default::default()
			},
		);

		let mut anc = superseded_ancestors(&g, "newest");
		anc.sort();
		assert_eq!(anc, vec!["mid".to_string(), "old".to_string()]);
		// A head with no Supersedes edges yields nothing.
		assert!(superseded_ancestors(&g, "old").is_empty());
	}

	#[test]
	fn add_reason_is_idempotent_on_adjacency() {
		let mut k = Kern::new("k", "");
		add_reason(&mut k, edge("a", "b"));
		add_reason(&mut k, edge("a", "b")); // same content-hash id
		add_reason(&mut k, edge("a", "b"));

		assert_eq!(k.reasons.len(), 1, "one reason in the map");
		assert_eq!(
			k.by_from.get("a").map(|v| v.len()),
			Some(1),
			"no dup in by_from"
		);
		assert_eq!(
			k.by_to.get("b").map(|v| v.len()),
			Some(1),
			"no dup in by_to"
		);
		assert_eq!(collect_reason_ids(&k, "a"), vec!["a->b".to_string()]);
	}

	#[test]
	fn remove_after_reobserve_fully_clears_adjacency() {
		let mut k = Kern::new("k", "");
		add_reason(&mut k, edge("a", "b"));
		add_reason(&mut k, edge("a", "b")); // re-observe
		remove_reason(&mut k, "a->b");

		assert!(k.reasons.is_empty(), "reason removed from map");
		assert!(
			k.by_from.get("a").map(|v| v.is_empty()).unwrap_or(true),
			"no stale id left in by_from"
		);
		assert!(
			collect_reason_ids(&k, "a").is_empty(),
			"no dangling edge id"
		);
	}

	#[test]
	fn move_entity_relocates_outgoing_and_stamps_cross_kern_targets() {
		let mut g = GraphGnn::new();
		let mut src = Kern::new("src", "");
		src.entities.insert("E".into(), ent("E", vec![]));
		src.entities.insert("X".into(), ent("X", vec![])); // third entity stays behind
		add_reason(&mut src, edge("E", "X")); // outgoing -> moves, stamp to_kern_id=src
		add_reason(&mut src, edge("E", "E")); // self-loop -> moves, no stamp
		add_reason(&mut src, edge("Y", "E")); // incoming -> stays in src, stamp to_kern_id=dst
		g.kerns.insert("src".into(), src);
		g.kerns.insert("dst".into(), Kern::new("dst", ""));

		move_entity(&mut g, "src", "dst", "E");

		let dst = g.kerns.get("dst").unwrap();
		let src = g.kerns.get("src").unwrap();
		assert!(dst.entities.contains_key("E"), "entity moved to dst");
		assert!(!src.entities.contains_key("E"), "entity gone from src");

		// Outgoing E->X moved and stamped with the SOURCE kern (X left behind there).
		assert_eq!(
			dst.reasons.get("E->X").map(|r| r.to_kern_id.as_str()),
			Some("src")
		);
		assert!(
			!src.reasons.contains_key("E->X"),
			"outgoing detached from src maps"
		);
		assert!(
			src.by_from.get("E").map(|v| v.is_empty()).unwrap_or(true),
			"src by_from[E] cleared"
		);
		// Self-loop E->E moved with both endpoints -> no cross-kern stamp.
		assert_eq!(
			dst.reasons.get("E->E").map(|r| r.to_kern_id.as_str()),
			Some("")
		);

		// Incoming Y->E stays in src (its `from` didn't move) but is stamped to dst.
		assert_eq!(
			src.reasons.get("Y->E").map(|r| r.to_kern_id.as_str()),
			Some("dst")
		);
		assert!(
			!dst.reasons.contains_key("Y->E"),
			"incoming reason not moved"
		);
	}

	#[test]
	fn move_entity_same_kern_or_missing_entity_is_noop() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert("E".into(), ent("E", vec![]));
		g.kerns.insert("k".into(), k);

		move_entity(&mut g, "k", "k", "E"); // same kern -> silent no-op
		assert!(g.kerns.get("k").unwrap().entities.contains_key("E"));
		move_entity(&mut g, "k", "dst", "ghost"); // missing entity -> silent no-op
		assert!(g.kerns.get("k").unwrap().entities.contains_key("E"));
	}

	#[test]
	fn remove_entity_cascades_through_reasons_and_hnsw_indices() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert("a".into(), ent("a", vec![1.0, 0.0]));
		k.entities.insert("b".into(), ent("b", vec![0.0, 1.0]));
		let mut e1 = edge("a", "b");
		e1.vector = vec![0.5, 0.5];
		let mut e2 = edge("b", "a");
		e2.vector = vec![0.4, 0.6];
		add_reason(&mut k, e1);
		add_reason(&mut k, e2);
		g.kerns.insert("k".into(), k);
		g.rebuild_index();
		assert_eq!(g.entity_idx.len(), 2, "two entities indexed");
		assert_eq!(g.reason_idx.len(), 2, "two reasons indexed");

		remove_entity(&mut g, "k", "a");

		let k = g.kerns.get("k").unwrap();
		assert!(!k.entities.contains_key("a"), "entity removed from map");
		assert!(!k.by_from.contains_key("a"), "by_from[a] purged");
		assert!(!k.by_to.contains_key("a"), "by_to[a] purged");
		assert!(
			k.reasons.is_empty(),
			"both incident reasons removed (a->b and b->a)"
		);
		assert!(
			collect_reason_ids(k, "b").is_empty(),
			"b left with no dangling edges"
		);
		assert_eq!(
			g.entity_idx.len(),
			1,
			"entity a purged from entity_idx, b remains"
		);
		assert_eq!(g.reason_idx.len(), 0, "both reasons purged from reason_idx");
	}

	#[test]
	fn remove_entity_fact_is_immune() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		let fact = Entity {
			id: "f".into(),
			kind: EntityKind::Fact,
			..Default::default()
		};
		k.entities.insert("f".into(), fact);
		g.kerns.insert("k".into(), k);

		remove_entity(&mut g, "k", "f");
		assert!(
			g.kerns.get("k").unwrap().entities.contains_key("f"),
			"facts are immune to removal"
		);
	}
}
