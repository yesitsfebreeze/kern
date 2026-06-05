//! Content-addressed CRDT merge for graph entities/reasons.
//!
//! Entity/reason ids are content hashes, so equal ids ⇒ identical immutable
//! content. Merge therefore joins only mutable metadata via conflict-free,
//! commutative, idempotent, monotone lattice operations: counters via
//! GCounter join, heat/confidence via max, status via the Active<Superseded
//! lattice, timestamps via min (creation) / max (activity).

use std::time::SystemTime;

use crate::base::constants;
use crate::base::graph::GraphGnn;
use crate::base::types::{Entity, EntityStatus, Reason};

fn join_max_time(local: &mut Option<SystemTime>, remote: Option<SystemTime>) -> bool {
	match (*local, remote) {
		(_, None) => false,
		(None, Some(r)) => {
			*local = Some(r);
			true
		}
		(Some(l), Some(r)) if r > l => {
			*local = Some(r);
			true
		}
		_ => false,
	}
}

fn join_min_time(local: &mut Option<SystemTime>, remote: Option<SystemTime>) -> bool {
	match (*local, remote) {
		(_, None) => false,
		(None, Some(r)) => {
			*local = Some(r);
			true
		}
		(Some(l), Some(r)) if r < l => {
			*local = Some(r);
			true
		}
		_ => false,
	}
}

/// CRDT join of `remote` into `local` (same content id assumed). Returns
/// whether `local` changed. Commutative, associative, idempotent, monotone.
pub fn merge_entity(local: &mut Entity, remote: &Entity) -> bool {
	let mut changed = local.access_count.merge(&remote.access_count);
	if remote.heat > local.heat {
		local.heat = remote.heat;
		changed = true;
	}
	if remote.conf_alpha > local.conf_alpha {
		local.conf_alpha = remote.conf_alpha;
		changed = true;
	}
	if remote.conf_beta > local.conf_beta {
		local.conf_beta = remote.conf_beta;
		changed = true;
	}
	// unlinked_count is local ingest bookkeeping, not convergent — left as-is.
	if remote.status == EntityStatus::Superseded && local.status != EntityStatus::Superseded {
		local.status = EntityStatus::Superseded;
		changed = true;
	}
	if !remote.superseded_by.is_empty() && remote.superseded_by > local.superseded_by {
		local.superseded_by = remote.superseded_by.clone();
		changed = true;
	}
	changed |= join_min_time(&mut local.created_at, remote.created_at);
	changed |= join_max_time(&mut local.accessed_at, remote.accessed_at);
	changed |= join_max_time(&mut local.updated_at, remote.updated_at);
	changed |= join_max_time(&mut local.heat_updated_at, remote.heat_updated_at);
	changed |= join_min_time(&mut local.valid_until, remote.valid_until);
	if changed {
		local.refresh_score();
	}
	changed
}

/// CRDT join for reasons (edge metadata).
pub fn merge_reason(local: &mut Reason, remote: &Reason) -> bool {
	let mut changed = local.traversal_count.merge(&remote.traversal_count);
	if remote.score > local.score {
		local.score = remote.score;
		changed = true;
	}
	changed
}

/// Merge a remote entity body into the designated `target_kern_id` (a
/// per-network `remote-*` phantom kern). Returns whether the graph changed.
///
/// SECURITY: this only ever touches `target_kern_id`. It must NOT search other
/// kerns for the id — a peer could otherwise forge an id that collides with a
/// local-origin entity (or another network's entity) and CRDT-merge
/// attacker-controlled metadata (status=Superseded, heat, confidence) into it,
/// or repoint the global entity index at a forgery. Behaviour by id ownership:
///   - id already in `target_kern_id` → CRDT-merge (genuine shared content);
///   - id owned by a *different* kern → reject (hijack attempt);
///   - id owned by no kern → insert, subject to a per-kern cap that bounds a
///     spamming peer.
pub fn merge_remote_entity(g: &mut GraphGnn, target_kern_id: &str, remote: Entity) -> bool {
	let host = g
		.kerns
		.iter()
		.find(|(_, k)| k.entities.contains_key(&remote.id))
		.map(|(kid, _)| kid.clone());
	match host {
		// Known shared content in the same network scope: CRDT-merge.
		Some(kid) if kid == target_kern_id => {
			if let Some(kern) = g.kerns.get_mut(&kid) {
				if let Some(local) = kern.entities.get_mut(&remote.id) {
					return merge_entity(local, &remote);
				}
			}
			false
		}
		// Id owned by another kern (local-origin or another network): a remote
		// peer must not be able to alter or hijack it.
		Some(other) => {
			tracing::warn!(
				target: "kern.merge",
				id = %crate::base::util::short_id(&remote.id),
				owner = %other,
				target = %target_kern_id,
				"remote entity id collides with an entity owned by another kern; rejected"
			);
			false
		}
		// Unknown id: insert into the target kern, subject to the cap.
		None => {
			let Some(kern) = g.kerns.get_mut(target_kern_id) else {
				tracing::warn!(target: "kern.merge", kern = %target_kern_id, "merge_remote_entity: target kern missing; entity dropped");
				return false;
			};
			if kern.entities.len() >= constants::GOSSIP_REMOTE_KERN_ENTITY_CAP {
				tracing::warn!(
					target: "kern.merge",
					kern = %target_kern_id,
					cap = constants::GOSSIP_REMOTE_KERN_ENTITY_CAP,
					"remote kern at entity cap; dropping new remote entity"
				);
				return false;
			}
			let id = remote.id.clone();
			kern.entities.insert(id.clone(), remote);
			// Borrow of `kern` ends here; index via &mut self below.
			g.index_entity(&id, target_kern_id);
			true
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{mk_entity, EntityKind, Kern};
	use std::time::{Duration, UNIX_EPOCH};

	fn t(secs: u64) -> Option<SystemTime> {
		Some(UNIX_EPOCH + Duration::from_secs(secs))
	}

	#[test]
	fn merge_is_monotonic() {
		// local heat 1.0, remote heat 5.0 -> 5.0
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		let remote = mk_entity("e1", "x", 5.0, EntityKind::Fact);
		let changed = merge_entity(&mut local, &remote);
		assert!(changed);
		assert_eq!(local.heat, 5.0);

		// reverse: local 5.0, remote 1.0 -> stays 5.0
		let mut local = mk_entity("e1", "x", 5.0, EntityKind::Fact);
		let remote = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		let changed = merge_entity(&mut local, &remote);
		assert!(!changed);
		assert_eq!(local.heat, 5.0);
	}

	#[test]
	fn merge_is_idempotent() {
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		let mut remote = mk_entity("e1", "x", 5.0, EntityKind::Fact);
		remote.access_count.increment("b", 2);
		remote.conf_alpha = 9.0;
		remote.accessed_at = t(100);
		remote.created_at = t(10);

		// first merge
		assert!(merge_entity(&mut local, &remote));
		let snap_heat = local.heat;
		let snap_alpha = local.conf_alpha;
		let snap_ac = local.access_count.value();
		let snap_acc = local.accessed_at;
		let snap_created = local.created_at;
		let snap_score = local.score;

		// second merge yields no change and identical fields
		let changed = merge_entity(&mut local, &remote);
		assert!(!changed);
		assert_eq!(local.heat, snap_heat);
		assert_eq!(local.conf_alpha, snap_alpha);
		assert_eq!(local.access_count.value(), snap_ac);
		assert_eq!(local.accessed_at, snap_acc);
		assert_eq!(local.created_at, snap_created);
		assert_eq!(local.score, snap_score);
	}

	#[test]
	fn merge_joins_access_count() {
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		local.access_count.increment("a", 1);
		let mut remote = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		remote.access_count.increment("b", 2);
		merge_entity(&mut local, &remote);
		assert_eq!(local.access_count.value(), 3);
	}

	#[test]
	fn merge_status_superseded_dominates() {
		// local Active + remote Superseded -> Superseded
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		let mut remote = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		remote.status = EntityStatus::Superseded;
		let changed = merge_entity(&mut local, &remote);
		assert!(changed);
		assert_eq!(local.status, EntityStatus::Superseded);

		// local Superseded + remote Active -> stays Superseded
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		local.status = EntityStatus::Superseded;
		let remote = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		merge_entity(&mut local, &remote);
		assert_eq!(local.status, EntityStatus::Superseded);
	}

	#[test]
	fn merge_created_at_takes_earliest_accessed_latest() {
		let mut local = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		local.created_at = t(100);
		local.accessed_at = t(100);
		let mut remote = mk_entity("e1", "x", 1.0, EntityKind::Fact);
		remote.created_at = t(50); // earlier
		remote.accessed_at = t(200); // later
		merge_entity(&mut local, &remote);
		assert_eq!(local.created_at, t(50)); // min
		assert_eq!(local.accessed_at, t(200)); // max
	}

	#[test]
	fn merge_remote_entity_inserts_then_merges() {
		let mut g = GraphGnn::new();
		let fallback = g.root.id.clone();

		let remote = mk_entity("eX", "x", 1.0, EntityKind::Fact);
		let changed = merge_remote_entity(&mut g, &fallback, remote);
		assert!(changed);
		// inserted into fallback kern
		assert!(g.kerns.get(&fallback).unwrap().entities.contains_key("eX"));
		assert_eq!(g.kern_of_entity("eX"), Some(fallback.as_str()));

		// merge same id again with higher heat -> existing updated, no dup
		let remote2 = mk_entity("eX", "x", 9.0, EntityKind::Fact);
		let changed = merge_remote_entity(&mut g, &fallback, remote2);
		assert!(changed);

		// count occurrences across all kerns: exactly one
		let total: usize = g
			.kerns
			.values()
			.filter(|k| k.entities.contains_key("eX"))
			.count();
		assert_eq!(total, 1);
		assert_eq!(
			g.kerns.get(&fallback).unwrap().entities.get("eX").unwrap().heat,
			9.0
		);
	}

	#[test]
	fn remote_cannot_hijack_id_owned_by_another_kern() {
		// SECURITY regression guard: a peer forging an id that collides with a
		// local-origin entity must NOT be able to merge metadata into it or
		// repoint the global index at a forgery.
		let mut g = GraphGnn::new();
		let local_kern = g.root.id.clone();
		// "eX" lives in the local/root kern.
		assert!(merge_remote_entity(
			&mut g,
			&local_kern,
			mk_entity("eX", "real", 1.0, EntityKind::Fact)
		));

		// A peer's per-network phantom kern.
		let phantom = "remote-netA-k1";
		g.register(Kern::new(phantom, &g.root.id));

		// Forged entity reuses the local id, trying to supersede + boost it.
		let mut forged = mk_entity("eX", "real", 9.0, EntityKind::Fact);
		forged.status = EntityStatus::Superseded;
		let changed = merge_remote_entity(&mut g, phantom, forged);

		assert!(!changed, "hijack must be rejected");
		let local = g.kerns.get(&local_kern).unwrap().entities.get("eX").unwrap();
		assert_eq!(local.status, EntityStatus::Active, "local status untouched");
		assert_eq!(local.heat, 1.0, "local heat untouched");
		assert!(
			!g.kerns.get(phantom).unwrap().entities.contains_key("eX"),
			"phantom kern must not gain the hijacked id"
		);
		assert_eq!(
			g.kern_of_entity("eX"),
			Some(local_kern.as_str()),
			"global index still points at the local owner"
		);
	}

	#[test]
	fn remote_kern_entity_cap_drops_new_ids_but_still_merges_known() {
		let mut g = GraphGnn::new();
		let phantom = "remote-netB-k1";
		g.register(Kern::new(phantom, &g.root.id));
		// Pre-fill the phantom kern to the cap with cheap placeholders.
		{
			let kern = g.kerns.get_mut(phantom).unwrap();
			for i in 0..constants::GOSSIP_REMOTE_KERN_ENTITY_CAP {
				kern.entities.insert(format!("f{i}"), Entity::default());
			}
		}
		// A brand-new remote id is dropped at the cap.
		let changed =
			merge_remote_entity(&mut g, phantom, mk_entity("newid", "x", 1.0, EntityKind::Fact));
		assert!(!changed, "new id past cap must be dropped");
		assert!(!g.kerns.get(phantom).unwrap().entities.contains_key("newid"));

		// An update to an EXISTING id still merges (not subject to the cap).
		let changed =
			merge_remote_entity(&mut g, phantom, mk_entity("f0", "x", 7.0, EntityKind::Fact));
		assert!(changed, "known id must still merge at cap");
		assert_eq!(g.kerns.get(phantom).unwrap().entities.get("f0").unwrap().heat, 7.0);
	}
}
