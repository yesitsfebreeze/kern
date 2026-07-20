use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::base::constants::{
	DISK_CONSOLIDATE_INTERVAL, DISK_CONSOLIDATE_MIN_DELTA, KERN_IDLE_SWEEP_EVERY, PULSE_DECAY,
	PULSE_THRESHOLD, STIGMERGY_GC_INTERVAL,
};
use crate::base::graph::GraphGnn;
use crate::base::heat::{self, HeatConfig};

use super::queue::{task, Queue, TaskKind};

// Unix-seconds of the last GC fan-out; single-flighted by compare_exchange.
static LAST_GC_AT_SECS: AtomicU64 = AtomicU64::new(0);

pub fn pulse(q: &Queue, g: &mut GraphGnn, kern_id: &str, strength: f64) {
	pulse_with_heat(q, g, kern_id, strength, &HeatConfig::default());
}

pub fn pulse_with_heat(
	q: &Queue,
	g: &mut GraphGnn,
	kern_id: &str,
	strength: f64,
	heat_cfg: &HeatConfig,
) {
	deposit_pulse(q, g, kern_id, strength, heat_cfg);
	if strength >= PULSE_THRESHOLD {
		maybe_enqueue_stigmergy_gc(q, g);
		maybe_enqueue_reembed(q, g);
		maybe_enqueue_disk_consolidate(q, g);
		maybe_enqueue_idle_sweep(q);
	}
}

// Unix-seconds of the last idle sweep; single-flighted by compare_exchange.
static LAST_IDLE_SWEEP_AT_SECS: AtomicU64 = AtomicU64::new(0);

fn maybe_enqueue_idle_sweep(q: &Queue) {
	if !claim_slot(&LAST_IDLE_SWEEP_AT_SECS, now_secs(), KERN_IDLE_SWEEP_EVERY) {
		return;
	}
	// Graph-global task: a fixed empty key means at most one is ever pending.
	q.enqueue(task(TaskKind::IdleSweep, ""));
}

fn now_secs() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0)
}

// Wins the cadence slot for exactly one caller; a fan-out cannot double-fire.
fn claim_slot(cell: &AtomicU64, now_secs: u64, interval: Duration) -> bool {
	let last = cell.load(Ordering::Relaxed);
	should_run_gc(now_secs, last, interval)
		&& cell
			.compare_exchange(last, now_secs, Ordering::AcqRel, Ordering::Relaxed)
			.is_ok()
}

pub fn should_run_gc(now_secs: u64, last_secs: u64, interval: Duration) -> bool {
	if now_secs == 0 || last_secs > now_secs {
		return false;
	}
	now_secs - last_secs >= interval.as_secs()
}

fn maybe_enqueue_stigmergy_gc(q: &Queue, g: &GraphGnn) {
	if !claim_slot(&LAST_GC_AT_SECS, now_secs(), STIGMERGY_GC_INTERVAL) {
		return;
	}
	for kern_id in g.kerns.keys() {
		q.enqueue(task(TaskKind::StigmergyGc, kern_id));
	}
}

// Unix-seconds of the last disk-consolidate fan-out; single-flighted by compare_exchange.
static LAST_CONSOLIDATE_AT_SECS: AtomicU64 = AtomicU64::new(0);

fn maybe_enqueue_disk_consolidate(q: &Queue, g: &GraphGnn) {
	let delta = g.pending_disk_delta_len();
	if delta < DISK_CONSOLIDATE_MIN_DELTA {
		return;
	}
	if !claim_slot(
		&LAST_CONSOLIDATE_AT_SECS,
		now_secs(),
		DISK_CONSOLIDATE_INTERVAL,
	) {
		return;
	}
	// Graph-global task: a fixed empty key means at most one is ever pending.
	q.enqueue(task(TaskKind::DiskConsolidate, ""));
}

fn maybe_enqueue_reembed(q: &Queue, g: &GraphGnn) {
	for (kern_id, k) in g.kerns.iter() {
		let dirty = k.entities.values().any(|e| e.dirty) || k.reasons.values().any(|r| r.dirty);
		if dirty {
			q.enqueue(task(TaskKind::Reembed, kern_id));
		}
	}
}

fn deposit_pulse(q: &Queue, g: &mut GraphGnn, kern_id: &str, strength: f64, heat_cfg: &HeatConfig) {
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

	let deposit = (heat_cfg.deposit_traversal as f64 * strength) as f32;
	if deposit > 0.0 {
		let now = SystemTime::now();
		if let Some(k) = g.kerns.get_mut(kern_id) {
			for tid in &entity_ids {
				if let Some(t) = k.entities.get_mut(tid) {
					t.heat = heat::deposit(
						t.heat,
						t.heat_updated_at,
						now,
						heat_cfg.half_life_secs,
						deposit,
					);
					t.heat_updated_at = Some(now);
				}
			}
		}
	}

	let reduced = strength * PULSE_DECAY;
	for child_id in &children {
		deposit_pulse(q, g, child_id, reduced, heat_cfg);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{mk_entity, EntityKind, Kern};
	use std::sync::Arc;

	fn cluster_kerns_after_pulse(strength: f64) -> Vec<String> {
		let mut g = GraphGnn::new();
		let mut p = Kern::new("p", "");
		p.children = vec!["c".into()];
		p.entities
			.insert("ep".into(), mk_entity("ep", "x", 0.0, EntityKind::Claim));
		let mut c = Kern::new("c", "p");
		c.entities
			.insert("ec".into(), mk_entity("ec", "y", 0.0, EntityKind::Claim));
		g.kerns.insert("p".into(), p);
		g.kerns.insert("c".into(), c);

		let q = Queue::new(64);
		deposit_pulse(
			&q,
			&mut g,
			"p",
			strength,
			&HeatConfig {
				half_life_secs: 3600,
				..HeatConfig::default()
			},
		);

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
		assert!(
			!should_run_gc(0, 0, iv),
			"unreadable clock (now=0) never sweeps"
		);
		assert!(
			!should_run_gc(50, 100, iv),
			"clock skew (last>now) never sweeps"
		);
		assert!(
			!should_run_gc(100, 50, iv),
			"50s elapsed < 100s interval -> no"
		);
		assert!(
			should_run_gc(150, 50, iv),
			"exactly the interval -> yes (>=)"
		);
		assert!(should_run_gc(200, 50, iv), "well past the interval -> yes");
	}

	#[test]
	fn pulse_decays_below_threshold_before_reaching_the_child() {
		let kerns = cluster_kerns_after_pulse(PULSE_THRESHOLD);
		assert!(kerns.contains(&"p".to_string()), "parent clusters");
		assert!(
			!kerns.contains(&"c".to_string()),
			"child is below threshold after one decay"
		);
	}

	#[test]
	fn pulse_reaches_the_child_when_strength_survives_one_decay() {
		let kerns = cluster_kerns_after_pulse(PULSE_THRESHOLD / PULSE_DECAY + 0.01);
		assert!(
			kerns.contains(&"c".to_string()),
			"child clusters when decay keeps it above threshold"
		);
	}

	#[test]
	fn pulse_deposits_using_the_configured_heat_settings_not_the_defaults() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("k", "");
		let mut e = mk_entity("e", "x", 0.0, EntityKind::Claim);
		e.heat = 8.0;
		e.heat_updated_at = Some(SystemTime::now() - Duration::from_secs(100));
		k.entities.insert("e".into(), e);
		g.kerns.insert("k".into(), k);

		let q = Queue::new(64);
		let cfg = HeatConfig {
			half_life_secs: 100,
			deposit_access: 1.0,
			deposit_traversal: 1.0,
		};
		pulse_with_heat(&q, &mut g, "k", 1.0, &cfg);

		let heat = g.kerns.get("k").unwrap().entities.get("e").unwrap().heat;
		assert!(
			(heat - 5.0).abs() < 0.05,
			"8 halved over the configured 100s half-life plus the configured 1.0 \
			 traversal deposit = ~5; the one-week default would give ~9, got {heat}"
		);
	}

	#[test]
	fn claim_slot_lets_exactly_one_caller_through_per_cadence() {
		let cell = AtomicU64::new(0);
		let iv = Duration::from_secs(60);

		assert!(claim_slot(&cell, 1_000, iv), "first call wins the slot");
		assert!(!claim_slot(&cell, 1_000, iv), "same second is gated");
		assert!(!claim_slot(&cell, 1_059, iv), "59s < 60s cadence is gated");
		assert!(claim_slot(&cell, 1_060, iv), "the next cadence wins again");
		assert!(!claim_slot(&cell, 0, iv), "unreadable clock never claims");
	}

	#[test]
	fn concurrent_claims_on_one_cadence_produce_exactly_one_winner() {
		use std::sync::atomic::AtomicUsize;

		static CELL: AtomicU64 = AtomicU64::new(0);
		let winners = Arc::new(AtomicUsize::new(0));
		let iv = Duration::from_secs(60);

		std::thread::scope(|s| {
			for _ in 0..16 {
				let winners = Arc::clone(&winners);
				s.spawn(move || {
					if claim_slot(&CELL, 5_000, iv) {
						winners.fetch_add(1, Ordering::Relaxed);
					}
				});
			}
		});

		assert_eq!(
			winners.load(Ordering::Relaxed),
			1,
			"a 16-way fan-out must not double-fire the sweep"
		);
	}

	#[test]
	fn reembed_is_enqueued_only_for_kerns_with_dirty_content() {
		let mut g = GraphGnn::new();
		let mut dirty = Kern::new("d", "");
		let mut e = mk_entity("e", "x", 0.0, EntityKind::Claim);
		e.dirty = true;
		dirty.entities.insert("e".into(), e);
		let mut clean = Kern::new("c", "");
		clean
			.entities
			.insert("e2".into(), mk_entity("e2", "y", 0.0, EntityKind::Claim));
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
		assert_eq!(
			reembed_kerns,
			vec!["d".to_string()],
			"only the kern with a dirty thought reembeds"
		);
	}
}
