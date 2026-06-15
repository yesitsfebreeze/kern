use super::graph::GraphGnn;
use super::types::{Kern, Reason};

/// All reason (edge) ids incident to `entity_id` in this kern — outgoing
/// (`by_from`) followed by incoming (`by_to`). Single source for the
/// edge-collection step shared by retrieval, the MCP tools, and the CLI.
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

pub fn add_reason(kern: &mut Kern, reason: Reason) {
	let id = reason.id.clone();
	let from = reason.from.clone();
	let to = reason.to.clone();
	// Index the adjacency lists only when the id is NEW. `reasons` is a map
	// (idempotent), but `by_from`/`by_to` are Vecs: re-adding the same edge id
	// (idempotent re-observe — Rephrase edges, move_entity round-trips,
	// re-ingest) would otherwise append a duplicate id, double-counting it in
	// `collect_reason_ids` and leaving a stale entry after `remove_reason`
	// (which removes only the first occurrence).
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

/// Relocates an entity (and its outgoing reasons) from `from_kern_id` to
/// `to_kern_id`, keeping every reverse-map index in sync.
///
/// # Cross-kern reason policy (outgoing-only)
///
/// The `Reason` data model tracks cross-kern targets via `to_kern_id` only —
/// there is no `from_kern_id` mirror. A move must therefore preserve the
/// invariant that the kern hosting a reason also hosts its `from` endpoint:
///
/// - **Outgoing reasons** (`r.from == E`): travel with the entity to the
///   destination kern. If the `to` endpoint is a third entity `X` still
///   living in the source kern, stamp `r.to_kern_id = from_kern_id` so the
///   reason knows its target lives elsewhere. Self-loops (`r.from == r.to
///   == E`) move with both endpoints intact and need no stamp.
/// - **Incoming reasons** (`r.to == E`, `r.from != E`): stay in the source
///   kern (their `from` endpoint has not moved). Stamp `r.to_kern_id =
///   to_kern_id` so the source-kern reason knows its target now lives in
///   the destination.
///
/// In both directions, stamping is skipped if `to_net_id` is already set
/// — that flag means the target lives on a remote node, and a local move
/// must not overwrite the remote-target annotation with a local kern id.
///
/// Cascade order (single pass, no lock re-acquisition):
/// 1. If `from_kern_id == to_kern_id`, the move is a silent no-op.
/// 2. The entity is removed from the source kern's `entities` map. If it
///    is not present, the call is a silent no-op (matches `remove_entity`).
/// 3. Outgoing reasons are detached from the source kern (`reasons`,
///    `by_from`, `by_to`), stamped per the policy above, and reattached to
///    the destination kern via `add_reason`. Incoming reasons stay in the
///    source kern's maps and are stamped in place.
/// 4. The entity is inserted into the destination kern.
/// 5. `entity_kern` is repointed to `to_kern_id` via `index_entity`. Only
///    relocated (outgoing) reasons have `reason_kern` repointed via
///    `index_reason`; incoming reasons' `reason_kern` stays at the source.
///
/// HNSW indices (`entity_idx`, `gnn_entity_idx`, `reason_idx`) and the
/// lexical index key on entity/reason id only and are unaffected by a
/// move; they are intentionally left untouched.
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

	// Partition reasons touching `entity_id` into outgoing (move) vs incoming
	// (stay). A self-loop `from == to == E` is treated as outgoing: both
	// endpoints land in the destination kern, so no stamping is needed.
	let (outgoing_rids, incoming_rids) = reasons_touching(src, entity_id);

	// Stamp incoming reasons in place: their `from` stays in the source kern,
	// but `to == E` now lives in `to_kern_id`. Only stamp when neither
	// cross-kern nor cross-net annotation is present — a non-empty
	// `to_net_id` means the reason already points at a remote node and
	// stamping a local kern id over it would lie about target location.
	for rid in &incoming_rids {
		if let Some(reason) = src.reasons.get_mut(rid) {
			if reason.to_kern_id.is_empty() && reason.to_net_id.is_empty() {
				reason.to_kern_id = to_kern_id.to_string();
			}
		}
	}

	// Detach outgoing reasons from source maps.
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
		// Stamp `to_kern_id = from_kern_id` when the `to` endpoint is a
		// third entity left behind in the source kern. Skip self-loops
		// (`to == entity_id`, which has moved with us), already-stamped
		// reasons, and reasons whose `to_net_id` flags a remote node —
		// a remote target stays remote regardless of the local move.
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

/// Removes an entity from the given kern and cascades the deletion through
/// every retrieval index that referenced it.
///
/// Cascade order (single pass, no lock re-acquisition):
/// 1. Fact entities are immune — early return preserving today's behaviour.
/// 2. All reasons sourced at or pointing to the entity are dropped from
///    `kern.reasons`, `kern.by_from`, `kern.by_to`, and from
///    `g.reason_idx` plus the `reason_kern` reverse map.
/// 3. The entity is dropped from `kern.entities`, from `g.entity_idx` and
///    `g.gnn_entity_idx`, and from the `entity_kern` reverse map.
/// 4. If a lexical index is installed it is purged for the entity id.
///
/// Infallible: a missing entity, missing kern, or empty index is a silent
/// no-op, matching the previous `&mut Kern` signature semantics.
pub fn remove_entity(g: &mut GraphGnn, kern_id: &str, id: &str) {
	let kern = match g.kerns.get_mut(kern_id) {
		Some(k) => k,
		None => return,
	};

	if let Some(t) = kern.entities.get(id) {
		if t.is_fact() {
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

/// Partition the reasons in `kern` that touch `entity_id` into
/// `(outgoing, incoming)`: outgoing = edges sourced at the entity (`by_from`);
/// incoming = edges pointing at it (`by_to`) that are not already outgoing, so a
/// self-loop counts once, as outgoing. Both lists are owned rid clones.
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

/// Remove the first occurrence of `s` from `vec` (if present).
///
/// The linear scan is intentional, not a missed optimization. `by_from`/`by_to`
/// values are an entity's adjacency list — its edge degree — which the daemon
/// keeps small: clustering caps a kern at a bounded entity count and edges are
/// pruned by decay/gc, so a list is tens of ids, not thousands. At that size a
/// `Vec` scan beats a `HashSet` on cache locality and avoids per-edge hashing.
/// `by_from`/`by_to` are also `Kern` fields serialized (serde) into the bincode
/// shards, so swapping `Vec<String>` for `HashSet<String>` is a persisted-format
/// change to weigh against the (nonexistent) hot-path win — not worth it.
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
	fn add_reason_is_idempotent_on_adjacency() {
		// Card #56: re-adding the same edge id must NOT duplicate it in the
		// by_from/by_to adjacency lists.
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
		// collect_reason_ids returns the edge exactly once.
		assert_eq!(collect_reason_ids(&k, "a"), vec!["a->b".to_string()]);
	}

	#[test]
	fn remove_after_reobserve_fully_clears_adjacency() {
		// With the idempotent add, a single remove leaves no stale dangling id.
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

	// ---- move_entity --------------------------------------------------------

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

	// ---- remove_entity ------------------------------------------------------

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
		// HNSW purge: a gone (b stays); both reasons gone.
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
