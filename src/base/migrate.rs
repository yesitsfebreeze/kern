//! One-shot migration from the legacy file-per-shard bincode tier into the LMDB
//! store in the same dir. Never destroys the source; no dual-read fallback.

use crate::base::persist::{load_legacy_dir, save_graph_into};
use crate::base::store::Store;

pub struct MigrateReport {
	pub kerns: usize,
	pub entities: usize,
}

/// Idempotent: re-running overwrites the store with the same legacy data.
pub fn migrate_dir(dir: &str) -> Result<MigrateReport, String> {
	let g = load_legacy_dir(dir).map_err(|e| format!("read legacy shards: {e}"))?;
	let kerns = g.map().len();
	let entities: usize = g.map().values().map(|k| k.entities.len()).sum();
	let store = Store::open(dir).map_err(|e| format!("open store: {e}"))?;
	save_graph_into(&store, &g).map_err(|e| format!("write store: {e}"))?;
	Ok(MigrateReport { kerns, entities })
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::persist::{load_dir, save_kern};
	use crate::base::types::{mk_entity, EntityKind, Kern};

	#[test]
	fn migrate_moves_legacy_shards_into_the_store() {
		let dir = tempfile::tempdir().unwrap();
		let d = dir.path().to_string_lossy().to_string();
		save_kern(&d, &Kern::new("root", "")).unwrap();
		let mut child = Kern::new("child", "root");
		let mut e = mk_entity("e1", "legacy fact", 1.0, EntityKind::Fact);
		e.vector = vec![0.2, -0.3, 0.4];
		child.entities.insert("e1".into(), e);
		save_kern(&d, &child).unwrap();

		let report = migrate_dir(&d).expect("migration succeeds");
		assert_eq!(report.kerns, 2, "root + child migrated");
		assert_eq!(report.entities, 1);

		let g = load_dir(&d).expect("store loads after migration");
		assert!(g.loaded("child").is_some(), "child present in the store");
		let be = &g.loaded("child").unwrap().entities["e1"];
		assert_eq!(be.text(), "legacy fact");
		assert!(
			(be.vector[0] - 0.2).abs() < 0.02,
			"vector survived (int8 on disk)"
		);
	}
}
