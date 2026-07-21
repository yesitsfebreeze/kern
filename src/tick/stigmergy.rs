use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use crate::base::log_throttle::LogThrottle;

use parking_lot::RwLock;

use crate::base::constants::{COLD_GC_AGE, COLD_HEAT_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::heat::{self, HeatConfig};
use crate::base::reason::remove_entity;
use crate::base::types::{Entity, EntityKind};

const SKEW_WARN_SECS: u64 = 300;
static CLOCK_SKEW: AtomicU64 = AtomicU64::new(0);
static UNSPILLED_DROPS: AtomicU64 = AtomicU64::new(0);
static SKEW_WARN: LogThrottle = LogThrottle::new(SKEW_WARN_SECS);

// Entities GC could not age because their timestamp is in the future. Nonzero
// means compaction is stalled on a clock problem, not on policy.
pub fn clock_skew_skips() -> u64 {
	CLOCK_SKEW.load(Ordering::Relaxed)
}

// Entities dropped with no cold store to spill into. Unrecoverable by design —
// an in-memory kern has nowhere to put them — so the count is the only trace,
// and the only thing separating that deployment from a durable one.
pub fn unspilled_drops() -> u64 {
	UNSPILLED_DROPS.load(Ordering::Relaxed)
}

// SECURITY: durable-kind immunity only holds for LOCAL kerns. A peer-supplied
// kind=Fact in a phantom kern would otherwise be permanently unreclaimable.
fn is_cold_victim(
	entity: &Entity,
	now: SystemTime,
	half_life_secs: u64,
	kern_is_remote: bool,
) -> bool {
	if !kern_is_remote
		&& !entity.is_superseded()
		&& matches!(entity.kind, EntityKind::Fact | EntityKind::Document)
	{
		return false;
	}
	// Stored heat is only ever refreshed on deposit, so an entity that went cold
	// long ago still carries its last hot value; age it before the comparison.
	let heat = heat::decayed(entity.heat, entity.heat_updated_at, now, half_life_secs);
	if (heat as f64) >= COLD_HEAT_THRESHOLD {
		return false;
	}
	let Some(last_touch) = entity.accessed_at.or(entity.created_at) else {
		return false;
	};
	match now.duration_since(last_touch) {
		Ok(age) => age > COLD_GC_AGE,
		// A timestamp in the future means an unreadable or rewound clock. Refusing
		// to reclaim is the safe side — but it is also indefinite: nothing else
		// bounds the hot graph, so a skewed clock stops compaction for as long as
		// it is skewed, and until now said nothing at all (ROADMAP item 7).
		Err(_) => {
			let total = CLOCK_SKEW.fetch_add(1, Ordering::Relaxed) + 1;
			if SKEW_WARN.allow() {
				tracing::warn!(
					target: "kern.gc",
					entity = %entity.id,
					total_skewed = total,
					"entity timestamp is in the future — GC cannot age it, so compaction is \
					 stalled for it; check the system clock (further skew counted, not logged)"
				);
			}
			false
		}
	}
}

pub fn run_gc(graph: &Arc<RwLock<GraphGnn>>, kern_id: &str, heat_cfg: &HeatConfig) {
	let mut g = graph.write();
	let kern = match g.kerns.get(kern_id) {
		Some(k) => k,
		None => return,
	};

	let now = SystemTime::now();
	let kern_is_remote = crate::base::merge::is_remote_kern_id(kern_id);
	let victims: Vec<String> = kern
		.entities
		.values()
		.filter(|t| is_cold_victim(t, now, heat_cfg.half_life_secs, kern_is_remote))
		.map(|t| t.id.clone())
		.collect();

	if victims.is_empty() {
		return;
	}

	// Spill-before-drop: eviction must never lose data — while a store is bound.
	let store = g.store();
	let kept = evict_victims(&mut g, kern_id, &victims, |e| match &store {
		Some(s) => s.cold_spill(e).is_ok(),
		// No cold store bound (in-memory kern): dropping IS the intended memory
		// bound, not a bug — there is nowhere to spill to. It is still a real loss,
		// and the spill-before-drop guarantee does not hold here, so count it rather
		// than let an in-memory deployment look like a durable one.
		None => {
			UNSPILLED_DROPS.fetch_add(1, Ordering::Relaxed);
			true
		}
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
		// never forced: GC does not get to punch through fact-immunity.
		remove_entity(g, kern_id, id, false);
	}
	kept
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::Kern;
	use std::time::Duration;

	const HL: u64 = 3600;

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
		assert!(is_cold_victim(
			&ent(EntityKind::Claim, 0.0, Some(old)),
			now,
			HL,
			false
		));
	}

	#[test]
	fn heat_above_threshold_is_preserved_even_when_old() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		let mut hot = ent(EntityKind::Claim, 1e9, Some(old));
		hot.heat_updated_at = Some(now);
		assert!(!is_cold_victim(&hot, now, HL, false));
	}

	#[test]
	fn stale_heat_decays_away_and_stops_shielding_the_entity() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		let mut once_hot = ent(EntityKind::Claim, 1e9, Some(old));
		once_hot.heat_updated_at = Some(old);
		assert!(
			is_cold_victim(&once_hot, now, HL, false),
			"heat last deposited a week ago has decayed below the threshold; \
			 raw stored heat must not grant permanent GC immunity"
		);
	}

	#[test]
	fn durable_kinds_are_never_collected() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(
			!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(old)), now, HL, false),
			"Fact preserved"
		);
		assert!(
			!is_cold_victim(&ent(EntityKind::Document, 0.0, Some(old)), now, HL, false),
			"Document preserved"
		);
	}

	// Cold-tier GC is the only bound on graph size; a remote Fact that kept durable
	// immunity would be permanently unreclaimable storage a peer chose to allocate.
	#[test]
	fn remote_durable_kinds_lose_immunity_and_are_reclaimed() {
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		for kind in [EntityKind::Fact, EntityKind::Document] {
			assert!(
				is_cold_victim(&ent(kind, 0.0, Some(old)), now, HL, true),
				"a cold, stale {kind:?} in a remote kern is reclaimable"
			);
			assert!(
				!is_cold_victim(&ent(kind, 0.0, Some(old)), now, HL, false),
				"the LOCAL {kind:?} keeps its immunity unchanged"
			);
		}
		assert!(
			!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(now)), now, HL, true),
			"a freshly-touched remote Fact is still spared — remoteness drops immunity, \
			 it does not bypass the staleness and heat gates"
		);
	}

	#[test]
	fn run_gc_reclaims_a_stale_remote_fact() {
		use crate::base::store::Store;

		let dir = tempfile::tempdir().unwrap();
		let store = Arc::new(Store::open(&dir.path().to_string_lossy()).unwrap());

		let old = SystemTime::now() - (COLD_GC_AGE + Duration::from_secs(1));
		let mut pinned = ent(EntityKind::Fact, 0.0, Some(old));
		pinned.id = "pinned".into();

		let mut g = GraphGnn::new();
		let mut k = Kern::new("remote-evilnet-k1", "");
		k.entities.insert("pinned".into(), pinned);
		g.kerns.insert("remote-evilnet-k1".into(), k);
		g.set_store(store.clone());

		let graph = Arc::new(RwLock::new(g));
		run_gc(&graph, "remote-evilnet-k1", &HeatConfig::default());

		assert!(
			!graph
				.read()
				.kerns
				.get("remote-evilnet-k1")
				.unwrap()
				.entities
				.contains_key("pinned"),
			"a peer cannot pin an unreclaimable row by setting kind=Fact"
		);
		assert!(
			store.cold_get("pinned").unwrap().is_some(),
			"spill-before-drop still holds for the remote fact"
		);
	}

	#[test]
	fn superseded_fact_loses_immunity_and_becomes_a_victim() {
		use crate::base::types::EntityStatus;
		let now = SystemTime::now();
		let old = now - (COLD_GC_AGE + Duration::from_secs(1));
		assert!(
			!is_cold_victim(&ent(EntityKind::Fact, 0.0, Some(old)), now, HL, false),
			"active Fact is immune even when stale"
		);
		let mut superseded = ent(EntityKind::Fact, 0.0, Some(old));
		superseded.status = EntityStatus::Superseded;
		assert!(
			is_cold_victim(&superseded, now, HL, false),
			"a superseded (invalidated) Fact is no longer immune"
		);
		let mut fresh_superseded = ent(EntityKind::Fact, 0.0, Some(now));
		fresh_superseded.status = EntityStatus::Superseded;
		assert!(
			!is_cold_victim(&fresh_superseded, now, HL, false),
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
		run_gc(&graph, "k", &HeatConfig::default());

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
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(now)), now, HL, false),
			"recently accessed"
		);
		assert!(
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, None), now, HL, false),
			"no timestamps at all"
		);
		let future = now + Duration::from_secs(3600);
		assert!(
			!is_cold_victim(&ent(EntityKind::Claim, 0.0, Some(future)), now, HL, false),
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
			is_cold_victim(&stale, now, HL, false),
			"old-but-never-queried is a victim"
		);
		let mut fresh = ent(EntityKind::Claim, 0.0, None);
		fresh.created_at = Some(now);
		assert!(
			!is_cold_victim(&fresh, now, HL, false),
			"fresh ingest is preserved"
		);
		let mut touched = ent(EntityKind::Claim, 0.0, Some(now));
		touched.created_at = Some(old);
		assert!(
			!is_cold_victim(&touched, now, HL, false),
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
		run_gc(&graph, "k", &HeatConfig::default());

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
	#[test]
	fn a_future_timestamp_is_not_reclaimed_and_is_counted() {
		// A rewound or unreadable clock makes every entity look untouchable, and
		// nothing else bounds the hot graph — so refusing to reclaim is right, and
		// refusing silently is the defect.
		let future = SystemTime::now() + Duration::from_secs(3600);
		let e = ent(EntityKind::Claim, 0.0, Some(future));

		let before = clock_skew_skips();
		let victim = is_cold_victim(
			&e,
			SystemTime::now(),
			HeatConfig::default().half_life_secs,
			false,
		);

		assert!(!victim, "a future timestamp must never be reclaimed");
		assert_eq!(
			clock_skew_skips(),
			before + 1,
			"and the stall must be countable, not silent"
		);
	}

	#[test]
	fn a_normal_old_entity_is_reclaimed_without_counting_skew() {
		let old = SystemTime::now() - (COLD_GC_AGE + Duration::from_secs(1));
		let e = ent(EntityKind::Claim, 0.0, Some(old));

		let before = clock_skew_skips();
		let victim = is_cold_victim(
			&e,
			SystemTime::now(),
			HeatConfig::default().half_life_secs,
			false,
		);

		assert!(victim, "precondition: a cold, old claim is a victim");
		assert_eq!(
			clock_skew_skips(),
			before,
			"a healthy clock must not read as a degradation"
		);
	}
	#[test]
	fn an_in_memory_kern_counts_what_it_drops_with_nowhere_to_spill() {
		// Spill-before-drop is a guarantee of a PERSISTED kern. With no store bound
		// there is nowhere to spill to and dropping is the intended memory bound —
		// but an in-memory deployment must not look durable, so the loss is counted.
		// Drives the real run_gc: a closure written in the test would prove nothing.
		let g = graph_with_cold_claim("victim");
		assert!(g.store().is_none(), "precondition: no cold store bound");
		let g = Arc::new(RwLock::new(g));

		let before = unspilled_drops();
		run_gc(&g, "k", &HeatConfig::default());

		assert!(
			!g.read()
				.kerns
				.get("k")
				.expect("kern k")
				.entities
				.contains_key("victim"),
			"precondition: the cold claim was actually evicted"
		);
		assert_eq!(
			unspilled_drops(),
			before + 1,
			"an unrecoverable drop must be countable, or in-memory reads as durable"
		);
	}
}
