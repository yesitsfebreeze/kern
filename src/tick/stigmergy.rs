//! Cold-path GC — the `forget()` half of evaporate → cool → forget; the
//! heat-decay half lives in `tick::pulse`.

use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::RwLock;

use crate::base::constants::{COLD_GC_AGE, COLD_HEAT_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::locks::write_recovered;
use crate::base::reason::remove_entity;
use crate::base::types::{Entity, EntityKind};

/// Cold-GC predicate: cold + stale + non-durable. Staleness reads `accessed_at`,
/// else `created_at`; no timestamp at all or a future clock (skew) preserves.
fn is_cold_victim(entity: &Entity, now: SystemTime) -> bool {
	// Fact/Document are immune UNLESS superseded — an invalidated fact is history
	// and may spill to the cold tier (invalidated ≠ deleted; the cold tier keeps it).
	if !entity.is_superseded() && matches!(entity.kind, EntityKind::Fact | EntityKind::Document) {
		return false;
	}
	if (entity.heat as f64) >= COLD_HEAT_THRESHOLD {
		return false;
	}
	let Some(last_touch) = entity.accessed_at.or(entity.created_at) else {
		return false;
	};
	match now.duration_since(last_touch) {
		Ok(age) => age > COLD_GC_AGE,
		Err(_) => false,
	}
}

/// Cold GC for one kern; victim policy in [`is_cold_victim`]. One write guard
/// for the whole kern; removal cascades edge cleanup via `remove_entity`.
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

	// Spill-before-drop: eviction never loses data. The store handle is cloned
	// out so the graph can keep mutating under the single write guard.
	let store = g.store();
	let kept = evict_victims(&mut g, kern_id, &victims, |e| match &store {
		Some(s) => s.cold_spill(e).is_ok(),
		// No cold store bound: dropping IS the intended memory bound, not a bug.
		None => true,
	});
	if kept > 0 {
		tracing::warn!(
			target: "kern.stigmergy",
			kern = %kern_id,
			kept,
			"cold spill failed for {kept} GC victim(s); kept hot, will retry next pass"
		);
	}
}

/// Drop each victim only after `spill` returns true (durably persisted); a failed
/// spill keeps the thought hot for the next pass. Returns the kept count.
fn evict_victims(
	g: &mut GraphGnn,
	kern_id: &str,
	victims: &[String],
	mut spill: impl FnMut(&Entity) -> bool,
) -> usize {
	let mut kept = 0usize;
	for id in victims {
		let victim = g
			.kerns
			.get(kern_id)
			.and_then(|k| k.entities.get(id))
			.cloned();
		if let Some(e) = victim {
			if !spill(&e) {
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
		Entity {
			id: "e".into(),
			kind,
			heat,
			accessed_at,
			..Default::default()
		}
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
		assert!(!is_cold_victim(
			&ent(EntityKind::Claim, 1e9, Some(old)),
			now
		));
	}

	#[test]
	fn durable_kinds_are_never_collected() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(
			!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(old)), now),
			"Fact preserved"
		);
		assert!(
			!is_cold_victim(&ent(EntityKind::Document, 0.0, Some(old)), now),
			"Document preserved"
		);
	}

	#[test]
	fn superseded_fact_loses_immunity_and_becomes_a_victim() {
		use crate::base::types::EntityStatus;
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(
			!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(old)), now),
			"active Fact is immune even when stale"
		);
		let mut superseded = ent(EntityKind::Fact, 0.0, Some(old));
		superseded.status = EntityStatus::Superseded;
		assert!(
			is_cold_victim(&superseded, now),
			"a superseded (invalidated) Fact is no longer immune"
		);
		// Losing immunity means "subject to GC", not "force-evicted".
		let mut fresh_superseded = ent(EntityKind::Fact, 0.0, Some(now));
		fresh_superseded.status = EntityStatus::Superseded;
		assert!(
			!is_cold_victim(&fresh_superseded, now),
			"a recently-touched superseded fact is still spared"
		);
	}

	#[test]
	fn run_gc_spills_superseded_fact_to_cold_while_active_fact_stays_immune() {
		use crate::base::store::Store;
		use crate::base::types::EntityStatus;
		use parking_lot::RwLock;
		use std::sync::Arc;

		let dir = tempfile::tempdir().unwrap();
		let store = Arc::new(Store::open(&dir.path().to_string_lossy()).unwrap());

		let old = SystemTime::now() - (COLD_GC_AGE + Duration::from_secs(1));
		let mut invalidated = ent(EntityKind::Fact, 0.0, Some(old));
		invalidated.id = "invalidated".into();
		invalidated.status = EntityStatus::Superseded;
		let mut active_fact = ent(EntityKind::Fact, 0.0, Some(old));
		active_fact.id = "active".into();

		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert("invalidated".into(), invalidated);
		k.entities.insert("active".into(), active_fact);
		g.kerns.insert("k".into(), k);
		g.set_store(store.clone());

		let graph = Arc::new(RwLock::new(g));
		run_gc(&graph, "k");

		let g = graph.read();
		let entities = &g.kerns.get("k").unwrap().entities;
		assert!(
			!entities.contains_key("invalidated"),
			"the superseded fact is evicted from the hot tier"
		);
		assert!(
			entities.contains_key("active"),
			"the active fact keeps its GC immunity"
		);
		assert!(
			store.cold_get("invalidated").unwrap().is_some(),
			"the invalidated fact was spilled to the cold tier (invalidated != deleted)"
		);
	}

	#[test]
	fn recent_untouched_or_clock_skewed_is_preserved() {
		let now = SystemTime::now();
		assert!(
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(now)), now),
			"recently accessed"
		);
		assert!(
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, None), now),
			"no timestamps at all"
		);
		let future = now + Duration::from_secs(3600);
		assert!(
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(future)), now),
			"clock skew"
		);
	}

	#[test]
	fn created_at_seeds_the_staleness_clock_for_never_accessed_thoughts() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		let mut stale = ent(EntityKind::Claim, 0.0, None);
		stale.created_at = Some(old);
		assert!(
			is_cold_victim(&stale, now),
			"old-but-never-queried is a victim"
		);
		let mut fresh = ent(EntityKind::Claim, 0.0, None);
		fresh.created_at = Some(now);
		assert!(!is_cold_victim(&fresh, now), "fresh ingest is preserved");
		let mut touched = ent(EntityKind::Claim, 0.0, Some(now));
		touched.created_at = Some(old);
		assert!(
			!is_cold_victim(&touched, now),
			"accessed_at takes precedence over created_at"
		);
	}

	#[test]
	fn run_gc_spills_stale_victim_to_cold_store_and_spares_facts() {
		use crate::base::store::Store;
		use parking_lot::RwLock;
		use std::sync::Arc;

		let dir = tempfile::tempdir().unwrap();
		let store = Arc::new(Store::open(&dir.path().to_string_lossy()).unwrap());

		let old = SystemTime::now() - (COLD_GC_AGE + Duration::from_secs(1));
		let mut victim = ent(EntityKind::Claim, 0.0, Some(old));
		victim.id = "victim".into();
		let mut fact = ent(EntityKind::Fact, 0.0, Some(old));
		fact.id = "fact".into();

		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		k.entities.insert("victim".into(), victim);
		k.entities.insert("fact".into(), fact);
		g.kerns.insert("k".into(), k);
		g.set_store(store.clone());

		let graph = Arc::new(RwLock::new(g));
		run_gc(&graph, "k");

		let g = graph.read();
		let entities = &g.kerns.get("k").unwrap().entities;
		assert!(
			!entities.contains_key("victim"),
			"stale cold claim is evicted from the hot tier"
		);
		assert!(
			entities.contains_key("fact"),
			"Facts are immune to cold GC even when stale"
		);
		let spilled = store.cold_get("victim").unwrap();
		assert!(
			spilled.is_some(),
			"the victim was spilled to the cold tier before the hot drop"
		);
		assert!(
			store.cold_get("fact").unwrap().is_none(),
			"the immune fact was never spilled"
		);
	}
}
