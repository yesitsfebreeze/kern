//! Autonomic cold-path garbage collection for the stigmergy substrate.
//!
//! Implements the loop promised in `docs/kern/stigmergy-self-improving.md`:
//! "unused pheromone evaporates → thought cools → automatic garbage collection
//! via `forget()`". This module is the `forget()` half — the heat-decay half
//! lives in `tick::pulse`.

use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::base::constants::{COLD_GC_AGE, COLD_HEAT_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::locks::write_recovered;
use crate::base::reason::remove_entity;
use crate::base::types::{Entity, EntityKind};

/// Pure cold-GC predicate: `true` iff `entity` should be dropped — its pheromone
/// has fully evaporated (`heat < COLD_HEAT_THRESHOLD`), it is genuinely abandoned
/// (`now - accessed_at > COLD_GC_AGE`), and it is not a durable kind (`Fact` /
/// `Document` are never auto-forgotten). An entity with no `accessed_at` is
/// treated as freshly created and preserved; a future `accessed_at` (clock skew)
/// is not stale. Split out from [`run_gc`]'s lock/store plumbing so the policy is
/// unit-testable in isolation.
fn is_cold_victim(entity: &Entity, now: SystemTime) -> bool {
	if matches!(entity.kind, EntityKind::Fact | EntityKind::Document) {
		return false;
	}
	if (entity.heat as f64) >= COLD_HEAT_THRESHOLD {
		return false;
	}
	let Some(accessed_at) = entity.accessed_at else {
		return false;
	};
	match now.duration_since(accessed_at) {
		Ok(age) => age > COLD_GC_AGE,
		Err(_) => false,
	}
}

/// Stigmergic cold-path GC for one kern.
///
/// Policy: a thought is dropped iff **all** of the following hold:
///
/// 1. `heat < COLD_HEAT_THRESHOLD` (pheromone has fully evaporated).
/// 2. `now - accessed_at > COLD_GC_AGE` (not just transiently quiet —
///    actually abandoned).
/// 3. `kind` is neither `Fact` nor `Document` (durable kinds per
///    `docs/kern/safety-architecture.md`; never auto-forgotten).
///
/// Removal goes through `base::reason::remove_entity`, which is the same
/// path used by the explicit `forget` command and which cascades edge
/// cleanup. We acquire the write guard exactly once for the whole kern —
/// no per-thought lock toggling.
///
/// Thoughts with no `accessed_at` timestamp are treated as recently created
/// (preserved); cold-but-untouched bookkeeping should not silently drop them.
pub fn run_gc(graph: &Arc<RwLock<GraphGnn>>, kern_id: &str) {
	let mut g = write_recovered(graph);
	let kern = match g.kerns.get(kern_id) {
		Some(k) => k,
		None => return,
	};

	let now = SystemTime::now();
	let victims: Vec<String> = kern
		.entities
		.values()
		.filter(|t| is_cold_victim(t, now))
		.map(|t| t.id.clone())
		.collect();

	if victims.is_empty() {
		return;
	}

	// Spill victims to the cold tier (in the store) before the hot drop, so
	// eviction never loses data immediately. `cold_spill` self-caps the tier
	// (drops oldest past COLD_MAX_ENTRIES), so no separate compaction pass is
	// needed. The store handle is cloned out (ref-counted) so we can keep
	// mutating the graph under the single write guard.
	let store = g.store();
	for id in &victims {
		if let Some(store) = &store {
			let victim = g.kerns.get(kern_id).and_then(|k| k.entities.get(id)).cloned();
			if let Some(e) = victim {
				let _ = store.cold_spill(&e);
			}
		}
		remove_entity(&mut g, kern_id, id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	fn ent(kind: EntityKind, heat: f32, accessed_at: Option<SystemTime>) -> Entity {
		Entity { id: "e".into(), kind, heat, accessed_at, ..Default::default() }
	}

	#[test]
	fn cold_old_claim_is_a_victim() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(old)), now));
	}

	#[test]
	fn heat_above_threshold_is_preserved_even_when_old() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(!is_cold_victim(&ent(EntityKind::Claim, 1e9, Some(old)), now));
	}

	#[test]
	fn durable_kinds_are_never_collected() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(old)), now), "Fact preserved");
		assert!(!is_cold_victim(&ent(EntityKind::Document, 0.0, Some(old)), now), "Document preserved");
	}

	#[test]
	fn recent_untouched_or_clock_skewed_is_preserved() {
		let now = SystemTime::now();
		// Cold but just accessed -> not yet abandoned.
		assert!(!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(now)), now), "recently accessed");
		// No accessed_at -> treated as freshly created.
		assert!(!is_cold_victim(&ent(EntityKind::Claim, 0.0, None), now), "never accessed");
		// accessed_at in the future (clock skew) -> not stale.
		let future = now + Duration::from_secs(3600);
		assert!(!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(future)), now), "clock skew");
	}
}
