//! Detached cold storage for evicted thoughts.
//!
//! Stigmergy GC spills cold, abandoned, non-durable entities here before
//! dropping them from the hot graph, so compaction never loses data. The
//! store is an append-only JSONL file under `<data_dir>/cold/`; the latest
//! line for an id wins on read (so a re-spilled, merged entity supersedes
//! an older copy). Retrieval rehydrates by id on demand (lazy-link).

use std::path::{Path, PathBuf};

use crate::base::types::Entity;

fn store_path(cold_dir: &Path) -> PathBuf {
	cold_dir.join("cold.jsonl")
}

/// Append `entity` to the cold store. Best-effort: creates the dir, ignores
/// write errors (a failed spill must not crash GC).
pub fn spill(cold_dir: &Path, entity: &Entity) {
	let _ = std::fs::create_dir_all(cold_dir);
	let line = match serde_json::to_string(entity) {
		Ok(s) => s,
		Err(_) => return,
	};
	use std::io::Write;
	match std::fs::OpenOptions::new()
		.create(true)
		.append(true)
		.open(store_path(cold_dir))
	{
		Ok(mut f) => {
			if writeln!(f, "{line}").is_err() {
				tracing::warn!(target: "kern.cold", "spill failed; entity not persisted to cold store");
			}
		}
		Err(_) => {
			tracing::warn!(target: "kern.cold", "spill failed; entity not persisted to cold store");
		}
	}
}

/// Rewrite the cold store keeping only the latest entry per id, bounding
/// file growth. Best-effort; a failure leaves the existing file intact.
pub fn compact(cold_dir: &Path) {
	let entities = load_all(cold_dir);
	if entities.is_empty() {
		return;
	}
	let tmp = cold_dir.join("cold.jsonl.tmp");
	let mut buf = String::new();
	for e in &entities {
		if let Ok(line) = serde_json::to_string(e) {
			buf.push_str(&line);
			buf.push('\n');
		}
	}
	if std::fs::write(&tmp, buf).is_ok() {
		let _ = std::fs::rename(&tmp, store_path(cold_dir));
	}
}

/// Load every entity from the cold store, latest-line-wins per id.
pub fn load_all(cold_dir: &Path) -> Vec<Entity> {
	let text = match std::fs::read_to_string(store_path(cold_dir)) {
		Ok(t) => t,
		Err(_) => return Vec::new(),
	};
	let mut by_id: std::collections::HashMap<String, Entity> = std::collections::HashMap::new();
	for line in text.lines() {
		if line.trim().is_empty() {
			continue;
		}
		if let Ok(e) = serde_json::from_str::<Entity>(line) {
			by_id.insert(e.id.clone(), e);
		}
	}
	by_id.into_values().collect()
}

/// Fetch one entity from the cold store by id (latest wins). None if absent.
pub fn get(cold_dir: &Path, id: &str) -> Option<Entity> {
	load_all(cold_dir).into_iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{
		Acl, ChunkPart, ChunkPartKind, Entity, EntityKind, EntityStatus, Source,
	};
	use crate::crdt::GCounter;

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

	#[test]
	fn spill_then_get_roundtrips() {
		let dir = tempfile::tempdir().unwrap();
		let e = mk_entity("a", "hello cold", 0.0, EntityKind::Claim);
		spill(dir.path(), &e);
		let got = get(dir.path(), "a").expect("entity should be in cold store");
		assert_eq!(got.id, "a");
		assert_eq!(got.text(), "hello cold");
	}

	#[test]
	fn latest_spill_wins() {
		let dir = tempfile::tempdir().unwrap();
		spill(dir.path(), &mk_entity("x", "v1", 1.0, EntityKind::Claim));
		spill(dir.path(), &mk_entity("x", "v2", 5.0, EntityKind::Claim));
		let got = get(dir.path(), "x").expect("entity should be present");
		assert_eq!(got.heat, 5.0);
		let all = load_all(dir.path());
		assert_eq!(all.iter().filter(|e| e.id == "x").count(), 1);
	}

	#[test]
	fn get_absent_is_none() {
		let dir = tempfile::tempdir().unwrap();
		assert!(get(dir.path(), "missing").is_none());
	}

	#[test]
	fn compact_dedups_to_latest() {
		let dir = tempfile::tempdir().unwrap();
		spill(dir.path(), &mk_entity("x", "v1", 1.0, EntityKind::Claim));
		spill(dir.path(), &mk_entity("x", "v2", 3.0, EntityKind::Claim));
		spill(dir.path(), &mk_entity("x", "v3", 5.0, EntityKind::Claim));
		spill(dir.path(), &mk_entity("y", "y1", 1.0, EntityKind::Claim));

		compact(dir.path());

		let raw = std::fs::read_to_string(store_path(dir.path())).unwrap();
		let lines = raw.lines().filter(|l| !l.trim().is_empty()).count();
		assert_eq!(lines, 2);
		assert_eq!(get(dir.path(), "x").unwrap().heat, 5.0);
	}
}
