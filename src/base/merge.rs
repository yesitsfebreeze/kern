//! Content-addressed CRDT merge for graph entities/reasons.
//!
//! Entity/reason ids are content hashes, so equal ids ⇒ identical immutable
//! content. Merge therefore joins only mutable metadata via conflict-free,
//! commutative, idempotent, monotone lattice operations: counters via
//! GCounter join, heat/confidence via max, status via the Active<Superseded
//! lattice, timestamps via min (creation) / max (activity).

use std::time::SystemTime;

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

/// Set-union + join: if an entity with `remote.id` already exists in any
/// kern, merge into it; otherwise insert into `fallback_kern_id` and index.
/// Returns whether the graph changed.
pub fn merge_remote_entity(g: &mut GraphGnn, fallback_kern_id: &str, remote: Entity) -> bool {
	// Find existing host kern, if any.
	let host = g
		.kerns
		.iter()
		.find(|(_, k)| k.entities.contains_key(&remote.id))
		.map(|(kid, _)| kid.clone());
	if let Some(kid) = host {
		if let Some(kern) = g.kerns.get_mut(&kid) {
			if let Some(local) = kern.entities.get_mut(&remote.id) {
				return merge_entity(local, &remote);
			}
		}
		false
	} else {
		let id = remote.id.clone();
		if let Some(kern) = g.kerns.get_mut(fallback_kern_id) {
			kern.entities.insert(id.clone(), remote);
			// Borrow of `kern` ends here; index via &mut self below.
			g.index_entity(&id, fallback_kern_id);
			true
		} else {
			tracing::warn!(target: "kern.merge", kern = %fallback_kern_id, "merge_remote_entity: fallback kern missing; entity dropped");
			false
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{
		Acl, ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source,
	};
	use crate::crdt::GCounter;
	use std::time::{Duration, UNIX_EPOCH};

	fn mk_entity(id: &str, text: &str, heat: f64, kind: EntityKind) -> Entity {
		let mut e = Entity {
			id: id.to_string(),
			root_id: String::new(),
			external_id: String::new(),
			superseded_by: String::new(),
			kind,
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
}
