use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::base::constants::{PULSE_DECAY, PULSE_THRESHOLD, STIGMERGY_GC_INTERVAL};
use crate::base::graph::GraphGnn;
use crate::base::heat::{self, HeatConfig};

use super::queue::{task, Queue, TaskKind};

/// Last unix-seconds at which `maybe_enqueue_stigmergy_gc` actually fanned
/// out a sweep. Single-flighted via `compare_exchange` so concurrent pulses
/// can never double-enqueue.
///
/// The *timing decision* lives in the pure [`should_run_gc`] (which takes
/// `now`/`last`/`interval` as args and is unit-tested directly), so this global
/// is only a thin single-flight latch — tests exercise the GC-cadence logic via
/// `should_run_gc` and never touch this static, keeping them parallel-safe.
static LAST_GC_AT_SECS: AtomicU64 = AtomicU64::new(0);

pub fn pulse(q: &Queue, g: &mut GraphGnn, kern_id: &str, strength: f64) {
	pulse_with_half_life(q, g, kern_id, strength, HeatConfig::default().half_life_secs);
	emit_stigmergy_snapshot(g, kern_id);
	// Below-threshold pulses are no-ops by contract; don't enqueue GC work
	// either. The next above-threshold pulse will trigger the sweep.
	if strength >= PULSE_THRESHOLD {
		maybe_enqueue_stigmergy_gc(q, g);
		maybe_enqueue_reembed(q, g);
	}
}

/// Pure decision: should `maybe_enqueue_stigmergy_gc` fan out a sweep now?
/// Returns true iff `interval` has fully elapsed since the last sweep.
/// `now_secs == 0` (callers couldn't read the clock) returns false.
/// `last_secs > now_secs` (clock skew) returns false — refuse to sweep on a
/// regressed clock to avoid amplifying time travel.
pub fn should_run_gc(now_secs: u64, last_secs: u64, interval: Duration) -> bool {
	if now_secs == 0 || last_secs > now_secs {
		return false;
	}
	now_secs - last_secs >= interval.as_secs()
}

/// Single-flight guard around `should_run_gc`: at most one pulse per
/// `STIGMERGY_GC_INTERVAL` enqueues `StigmergyGc` for every kern. Pending
/// per-kern dedup is owned by `Queue::enqueue` via the existing TaskKey
/// map, so a slow handler can't pile up duplicates either.
fn maybe_enqueue_stigmergy_gc(q: &Queue, g: &GraphGnn) {
	let now_secs = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let last = LAST_GC_AT_SECS.load(Ordering::Relaxed);
	if !should_run_gc(now_secs, last, STIGMERGY_GC_INTERVAL) {
		return;
	}
	if LAST_GC_AT_SECS
		.compare_exchange(last, now_secs, Ordering::AcqRel, Ordering::Relaxed)
		.is_err()
	{
		return;
	}
	for kern_id in g.kerns.keys() {
		q.enqueue(task(TaskKind::StigmergyGc, kern_id));
	}
}

/// Enqueue a `Reembed` sweep for every kern that has a dirty (edited) thought or
/// reason, so edits made directly in the graph get re-embedded even without an
/// explicit trigger (e.g. after a restart). Dedup is owned by `Queue::enqueue`.
fn maybe_enqueue_reembed(q: &Queue, g: &GraphGnn) {
	for (kern_id, k) in g.kerns.iter() {
		let dirty = k.entities.values().any(|e| e.dirty) || k.reasons.values().any(|r| r.dirty);
		if dirty {
			q.enqueue(task(TaskKind::Reembed, kern_id));
		}
	}
}

/// Emit one `kern::metrics` tracing event summarising heat distribution at the
/// pulsed kern. Called from the public `pulse` entrypoint only — never from
/// recursive `pulse_with_half_life` frames — so each pulse tree yields exactly
/// one snapshot.
fn emit_stigmergy_snapshot(g: &GraphGnn, kern_id: &str) {
	let Some(k) = g.kerns.get(kern_id) else {
		return;
	};
	let snap = crate::metrics::snapshot_heat(k.entities.values());
	if snap.n > 0 {
		tracing::info!(
			target: "kern::metrics",
			gini = snap.gini,
			max_heat = snap.max_heat,
			n = snap.n,
			"stigmergy_snapshot"
		);
	}
}

pub fn pulse_with_half_life(
	q: &Queue,
	g: &mut GraphGnn,
	kern_id: &str,
	strength: f64,
	half_life_secs: u64,
) {
	if strength < PULSE_THRESHOLD {
		return;
	}
	let (children, has_thoughts, entity_ids): (Vec<String>, bool, Vec<String>) = {
		let Some(k) = g.kerns.get(kern_id) else {
			return;
		};
		(
			k.children.clone(),
			!k.entities.is_empty(),
			k.entities.keys().cloned().collect(),
		)
	};

	if has_thoughts {
		q.enqueue(task(TaskKind::Cluster, kern_id));
	}

	let deposit = (HeatConfig::default().deposit_traversal as f64 * strength) as f32;
	if deposit > 0.0 {
		let now = SystemTime::now();
		if let Some(k) = g.kerns.get_mut(kern_id) {
			for tid in &entity_ids {
				if let Some(t) = k.entities.get_mut(tid) {
					t.heat = heat::deposit(t.heat, t.heat_updated_at, now, half_life_secs, deposit);
					t.heat_updated_at = Some(now);
				}
			}
		}
	}

	let reduced = strength * PULSE_DECAY;
	for child_id in &children {
		pulse_with_half_life(q, g, child_id, reduced, half_life_secs);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{mk_entity, EntityKind, Kern};

	fn cluster_kerns_after_pulse(strength: f64) -> Vec<String> {
		let mut g = GraphGnn::new();
		let mut p = Kern::new("p", "");
		p.children = vec!["c".into()];
		p.entities.insert("ep".into(), mk_entity("ep", "x", 0.0, EntityKind::Claim));
		let mut c = Kern::new("c", "p");
		c.entities.insert("ec".into(), mk_entity("ec", "y", 0.0, EntityKind::Claim));
		g.kerns.insert("p".into(), p);
		g.kerns.insert("c".into(), c);

		let q = Queue::new(64);
		pulse_with_half_life(&q, &mut g, "p", strength, 3600);

		let mut rx = q.take_receiver().unwrap();
		let mut kerns = Vec::new();
		while let Ok(t) = rx.try_recv() {
			if matches!(t.kind, TaskKind::Cluster) {
				kerns.push(t.kern_id.clone());
			}
		}
		kerns
	}

	#[test]
	fn should_run_gc_gates_on_clock_validity_and_elapsed_interval() {
		let iv = Duration::from_secs(100);
		assert!(!should_run_gc(0, 0, iv), "unreadable clock (now=0) never sweeps");
		assert!(!should_run_gc(50, 100, iv), "clock skew (last>now) never sweeps");
		assert!(!should_run_gc(100, 50, iv), "50s elapsed < 100s interval -> no");
		assert!(should_run_gc(150, 50, iv), "exactly the interval -> yes (>=)");
		assert!(should_run_gc(200, 50, iv), "well past the interval -> yes");
	}

	#[test]
	fn pulse_decays_below_threshold_before_reaching_the_child() {
		// At exactly the threshold the parent pulses, but one decay (×PULSE_DECAY)
		// drops the child below it, so no child Cluster task is enqueued.
		let kerns = cluster_kerns_after_pulse(PULSE_THRESHOLD);
		assert!(kerns.contains(&"p".to_string()), "parent clusters");
		assert!(!kerns.contains(&"c".to_string()), "child is below threshold after one decay");
	}

	#[test]
	fn pulse_reaches_the_child_when_strength_survives_one_decay() {
		// Strong enough that strength*PULSE_DECAY still clears the threshold.
		let kerns = cluster_kerns_after_pulse(PULSE_THRESHOLD / PULSE_DECAY + 0.01);
		assert!(kerns.contains(&"c".to_string()), "child clusters when decay keeps it above threshold");
	}

	#[test]
	fn reembed_is_enqueued_only_for_kerns_with_dirty_content() {
		let mut g = GraphGnn::new();
		let mut dirty = Kern::new("d", "");
		let mut e = mk_entity("e", "x", 0.0, EntityKind::Claim);
		e.dirty = true;
		dirty.entities.insert("e".into(), e);
		let mut clean = Kern::new("c", "");
		clean.entities.insert("e2".into(), mk_entity("e2", "y", 0.0, EntityKind::Claim));
		g.kerns.insert("d".into(), dirty);
		g.kerns.insert("c".into(), clean);

		let q = Queue::new(64);
		maybe_enqueue_reembed(&q, &g);

		let mut rx = q.take_receiver().unwrap();
		let mut reembed_kerns = Vec::new();
		while let Ok(t) = rx.try_recv() {
			if matches!(t.kind, TaskKind::Reembed) {
				reembed_kerns.push(t.kern_id.clone());
			}
		}
		assert_eq!(reembed_kerns, vec!["d".to_string()], "only the kern with a dirty thought reembeds");
	}
}
