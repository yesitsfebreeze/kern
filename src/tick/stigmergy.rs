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

	// Spill each victim to the cold tier before the hot drop, so eviction never
	// loses data. `cold_spill` self-caps the tier (drops oldest past
	// COLD_MAX_ENTRIES), so no separate compaction pass is needed. The store
	// handle is cloned out (ref-counted) so we can keep mutating the graph under
	// the single write guard.
	let store = g.store();
	let kept = evict_victims(&mut g, kern_id, &victims, |e| match &store {
		// A thought is only safe to drop once it is durably in the cold store.
		Some(s) => s.cold_spill(e).is_ok(),
		// No cold store bound: a pure in-memory graph has nowhere to spill, so
		// dropping is the intended bound on memory (nothing to persist).
		None => true,
	});
	if kept > 0 {
		// Fail-soft kept these hot rather than dropping them unpersisted. Surface
		// it: a persistently failing cold store would otherwise silently let hot
		// memory grow with no signal that GC is stalled on a broken store.
		tracing::warn!(
			target: "kern.stigmergy",
			kern = %kern_id,
			kept,
			"cold spill failed for {kept} GC victim(s); kept hot, will retry next pass"
		);
	}
}

/// Drop each victim from the hot graph, but only after `spill` confirms it is
/// durably persisted (`spill` returns `true`). If a spill fails the thought is
/// kept hot and retried on the next GC pass — we never drop an unpersisted
/// thought, even if the cold store is transiently erroring. Returns the number
/// of victims kept because their spill failed (0 in the healthy path). Split out
/// from [`run_gc`] so the drop-iff-persisted invariant is unit-testable without a
/// failing store.
fn evict_victims(
	g: &mut GraphGnn,
	kern_id: &str,
	victims: &[String],
	mut spill: impl FnMut(&Entity) -> bool,
) -> usize {
	let mut kept = 0usize;
	for id in victims {
		let victim = g.kerns.get(kern_id).and_then(|k| k.entities.get(id)).cloned();
		if let Some(e) = victim {
			if !spill(&e) {
				// Spill failed → leave the thought in the hot tier for retry.
				kept += 1;
				continue;
			}
		}
		remove_entity(g, kern_id, id);
	}
	kept
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::Kern;
	use std::time::Duration;

	fn ent(kind: EntityKind, heat: f32, accessed_at: Option<SystemTime>) -> Entity {
		Entity { id: "e".into(), kind, heat, accessed_at, ..Default::default() }
	}

	fn graph_with_cold_claim(id: &str) -> GraphGnn {
		let old = SystemTime::now() - (COLD_GC_AGE + Duration::from_secs(1));
		let mut e = ent(EntityKind::Claim, 0.0, Some(old));
		e.id = id.into();
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert(id.into(), e);
		g.kerns.insert("k".into(), k);
		g
	}

	#[test]
	fn evict_keeps_victim_hot_when_spill_fails() {
		// The data-loss guard: if the cold spill does not durably succeed, the
		// thought must stay in the hot tier (retried next pass), never dropped.
		let mut g = graph_with_cold_claim("victim");
		let kept = evict_victims(&mut g, "k", &["victim".to_string()], |_| false);
		assert_eq!(kept, 1, "the failed-spill victim is counted as kept");
		assert!(
			g.kerns.get("k").unwrap().entities.contains_key("victim"),
			"spill failure must NOT drop the thought"
		);
	}

	#[test]
	fn evict_drops_victim_once_spill_succeeds() {
		let mut g = graph_with_cold_claim("victim");
		let kept = evict_victims(&mut g, "k", &["victim".to_string()], |_| true);
		assert_eq!(kept, 0, "a successful spill keeps nothing back");
		assert!(
			!g.kerns.get("k").unwrap().entities.contains_key("victim"),
			"a durably-spilled thought is dropped from the hot tier"
		);
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
