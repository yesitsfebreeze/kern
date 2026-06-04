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
use crate::base::types::EntityKind;

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
pub fn run_gc(
	graph: &Arc<RwLock<GraphGnn>>,
	kern_id: &str,
	cold_dir: Option<&std::path::Path>,
) {
	let mut g = write_recovered(graph);
	let kern = match g.kerns.get(kern_id) {
		Some(k) => k,
		None => return,
	};

	let now = SystemTime::now();
	let mut victims: Vec<String> = Vec::new();
	for thought in kern.entities.values() {
		if matches!(thought.kind, EntityKind::Fact | EntityKind::Document) {
			continue;
		}
		if (thought.heat as f64) >= COLD_HEAT_THRESHOLD {
			continue;
		}
		let accessed_at = match thought.accessed_at {
			Some(t) => t,
			None => continue,
		};
		let age = match now.duration_since(accessed_at) {
			Ok(d) => d,
			// clock skew (accessed_at in the future) → not stale
			Err(_) => continue,
		};
		if age <= COLD_GC_AGE {
			continue;
		}
		victims.push(thought.id.clone());
	}

	if victims.is_empty() {
		return;
	}

	for id in &victims {
		if let Some(dir) = cold_dir {
			// Spill the victim to the detached cold tier before the hot drop,
			// so self-compaction never permanently loses data. Clone it out
			// of the kern while we still hold the single write guard.
			let victim = g.kerns.get(kern_id).and_then(|k| k.entities.get(id)).cloned();
			if let Some(e) = victim {
				crate::base::cold::spill(dir, &e);
			}
		}
		remove_entity(&mut g, kern_id, id);
	}

	// Bound cold-store growth: dedup to the latest entry per id once per sweep.
	if let Some(dir) = cold_dir {
		if !victims.is_empty() {
			crate::base::cold::compact(dir);
		}
	}
}
