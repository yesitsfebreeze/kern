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
	kern.reasons.insert(id.clone(), reason);
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
	let mut outgoing_rids: Vec<String> = Vec::new();
	let mut incoming_rids: Vec<String> = Vec::new();
	if let Some(from_rids) = src.by_from.get(entity_id) {
		outgoing_rids.extend(from_rids.iter().cloned());
	}
	if let Some(to_rids) = src.by_to.get(entity_id) {
		for rid in to_rids {
			// Self-loops already captured via `by_from`; skip the dup.
			if !outgoing_rids.iter().any(|x| x == rid) {
				incoming_rids.push(rid.clone());
			}
		}
	}

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

	let mut rids = Vec::new();
	if let Some(from_rids) = kern.by_from.get(id) {
		rids.extend(from_rids.clone());
	}
	if let Some(to_rids) = kern.by_to.get(id) {
		rids.extend(to_rids.clone());
	}
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

fn remove_string_from_vec(vec: Option<&mut Vec<String>>, s: &str) {
	if let Some(v) = vec {
		if let Some(pos) = v.iter().position(|x| x == s) {
			v.remove(pos);
		}
	}
}
