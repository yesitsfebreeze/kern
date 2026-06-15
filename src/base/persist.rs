// SLICE A NOTE — Entity rename is a clean break. The on-disk bincode layout
// changed (`Thought` -> `Entity`, `ThoughtKind` -> `EntityKind` + `EntityStatus`,
// `SourceRef` -> typed `Source` enum, `Kern.thoughts` -> `Kern.entities`,
// `created_at` moved off `Source` onto `Entity`). Old saved DBs are NOT
// migrated — they must be regenerated. CLAUDE.md mandates "no compat".
//
// ENCRYPTION-AT-REST POSTURE: snapshots are written as PLAINTEXT bincode
// (`atomic_write` below does a tmp-write + fsync + atomic rename — durability,
// not confidentiality). This layer intentionally does no encryption. Protecting
// the `.kern` data dir is a DEPLOYMENT-layer concern (full-disk / volume
// encryption, filesystem ACLs); do not store secrets in kern expecting the file
// layer to guard them.
use super::graph::{migrate_root_id, GraphGnn};
use super::store::bincode_cfg;
use super::types::Kern;
use super::util;
use crate::quant::QuantizationMode;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
	#[error("io: {0}")]
	Io(#[from] std::io::Error),
	#[error("bincode encode: {0}")]
	BincodeEncode(#[from] bincode::error::EncodeError),
	#[error("bincode decode: {0}")]
	BincodeDecode(#[from] bincode::error::DecodeError),
	#[error("missing node: {0}")]
	MissingNode(String),
	#[error("atomic rename {tmp:?} -> {dst:?}: {source}")]
	TmpRename {
		tmp: PathBuf,
		dst: PathBuf,
		#[source]
		source: std::io::Error,
	},
}

/// Append a literal suffix to a path (whole filename, not the extension).
/// Stays in the same directory so a subsequent `rename` is atomic.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
	let mut p = path.as_os_str().to_owned();
	p.push(suffix);
	PathBuf::from(p)
}

/// Tmp-file convention: append `.tmp` to the final path. Same directory
/// (same volume) so `rename` is atomic on both Windows and Unix.
fn tmp_path(path: &Path) -> PathBuf {
	append_suffix(path, ".tmp")
}

/// Crash-atomic write: write `data` to `{path}.tmp`, fsync, then rename.
/// Existing `.tmp` siblings are overwritten (treated as stale).
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), PersistError> {
	let tmp = tmp_path(path);
	{
		let mut f = fs::File::create(&tmp)?;
		f.write_all(data)?;
		f.sync_all()?;
	}
	if let Err(source) = fs::rename(&tmp, path) {
		// best-effort cleanup; surface original rename error
		let _ = fs::remove_file(&tmp);
		return Err(PersistError::TmpRename {
			tmp,
			dst: path.to_path_buf(),
			source,
		});
	}
	Ok(())
}

/// Sweep stray `*.tmp` files in `dir` left by a crashed prior write.
/// Logs and deletes; never recovers from a tmp.
fn sweep_stale_tmp(dir: &Path) {
	let entries = match fs::read_dir(dir) {
		Ok(e) => e,
		Err(_) => return,
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.extension() == Some(OsStr::new("tmp")) {
			tracing::warn!(
				target: "kern::persist",
				tmp = %path.display(),
				"removing stale .tmp file from incomplete prior write"
			);
			let _ = fs::remove_file(&path);
		}
	}
}

#[derive(Serialize, Deserialize)]
struct QuantMeta {
	mode: QuantizationMode,
}

fn quant_dir_sidecar(dir: &str) -> PathBuf {
	Path::new(dir).join("_quant.meta")
}

fn read_quant_mode(sidecar: &Path) -> QuantizationMode {
	let data = match fs::read(sidecar) {
		Ok(d) => d,
		Err(_) => return QuantizationMode::None,
	};
	bincode::serde::decode_from_slice::<QuantMeta, _>(&data, bincode_cfg())
		.map(|(m, _)| m.mode)
		.unwrap_or(QuantizationMode::None)
}

pub fn save_kern(dir: &str, kern: &Kern) -> Result<(), PersistError> {
	let path = Path::new(dir).join(format!("{}.kern", kern.id));
	let data = bincode::serde::encode_to_vec(kern, bincode_cfg())?;
	atomic_write(&path, &data)?;
	Ok(())
}

pub fn load_kern(dir: &str, id: &str) -> Result<Kern, PersistError> {
	let path = Path::new(dir).join(format!("{id}.kern"));
	let data = fs::read(path)?;
	let (mut kern, _): (Kern, _) = bincode::serde::decode_from_slice(&data, bincode_cfg())?;
	backfill_created_at(&mut kern);
	Ok(kern)
}

/// DEPRECATION HORIZON: this is legacy-read scaffolding, not a permanent
/// migration. It backfills `created_at` for entities decoded from old file
/// shards that predate the field. It silently mutates loaded data on every
/// restart, which is acceptable ONLY because it is part of the file-shard
/// *reader* that the redb-`Store` migration is retiring: per
/// `docs/superpowers/plans/2026-06-10-redb-store.md` (Step 1), `load_kern` /
/// `load_dir` / `backfill_created_at` move into `migrate.rs::read_legacy_dir`
/// and this whole path is deleted once stores are the only on-disk format.
/// Do NOT extend it — new fields use serde `#[serde(default)]`, not a backfill.
fn backfill_created_at(kern: &mut Kern) {
	let now = std::time::SystemTime::now();
	for t in kern.entities.values_mut() {
		if t.created_at.is_none() {
			t.created_at = Some(now);
		}
	}
}

/// Load the graph from the embedded LMDB store under `dir`. Opens the store once
/// and binds it to the returned graph for the lazy-load / persist paths. An empty
/// or root-less store yields a fresh graph bound to the (now-open) store, so the
/// very first run on a new project persists correctly.
pub fn load_dir(dir: &str) -> Result<GraphGnn, crate::base::store::StoreError> {
	use crate::base::store::Store;
	use std::sync::Arc;

	let store = Store::open(dir)?;
	let (mut kerns, mut network_id, quant_mode) = store.load_all_kerns()?;
	if network_id.is_empty() {
		network_id = util::uuid_v4();
	}
	let store = Arc::new(store);

	if !kerns.contains_key("root") {
		let mut g = GraphGnn::new();
		g.data_dir = dir.to_string();
		g.set_store(store);
		return Ok(g);
	}

	for k in kerns.values_mut() {
		migrate_root_id(k, &network_id);
		backfill_created_at(k);
	}
	let root = kerns
		.get("root")
		.cloned()
		.expect("root presence checked above");
	let mut g = GraphGnn::from_saved_with_mode(
		root,
		network_id,
		dir.to_string(),
		kerns,
		std::collections::HashSet::new(),
		quant_mode,
	);
	g.set_store(store);
	Ok(g)
}

/// Legacy file-per-shard reader, retained solely for the one-shot `kern migrate`
/// path (see `crate::base::migrate`). New loads go through the store-backed
/// [`load_dir`]. Reads every `<id>.kern` bincode shard under `dir`.
pub fn load_legacy_dir(dir: &str) -> Result<GraphGnn, PersistError> {
	sweep_stale_tmp(Path::new(dir));
	let mut root = load_kern(dir, "root")?;
	let mut network_id = load_network_id(dir);
	if network_id.is_empty() {
		network_id = util::uuid_v4();
	}
	migrate_root_id(&mut root, &network_id);

	let mut kerns = HashMap::new();
	let root_id = root.id.clone();
	kerns.insert(root_id.clone(), root);

	let unloaded = std::collections::HashSet::new();

	// Enumerate sibling shard ids first (cheap directory walk), then decode them
	// in parallel. A real graph can hold hundreds of thousands of `.kern` files;
	// a sequential read+bincode-decode of each one made `load_dir` an O(shards)
	// multi-minute hang that blocked every CLI command and the daemon's startup
	// reap. The decode is pure CPU+IO per file with no cross-shard state, so it
	// fans out cleanly across rayon's pool — wall time drops by ~core count.
	let ids: Vec<String> = fs::read_dir(dir)?
		.filter_map(Result::ok)
		.filter_map(|entry| {
			let name = entry.file_name().to_string_lossy().to_string();
			let id = name.strip_suffix(".kern")?;
			if id == root_id || id == "_meta" {
				return None;
			}
			Some(id.to_string())
		})
		.collect();

	let decoded: Vec<Result<Kern, (String, PersistError)>> = ids
		.par_iter()
		.map(|id| match load_kern(dir, id) {
			Ok(mut k) => {
				migrate_root_id(&mut k, &network_id);
				Ok(k)
			}
			Err(e) => Err((id.clone(), e)),
		})
		.collect();

	let mut skipped = 0usize;
	for result in decoded {
		match result {
			Ok(k) => {
				kerns.insert(k.id.clone(), k);
			}
			// A corrupt/unreadable sibling must not vanish silently: warn so the
			// orphaned data is visible rather than producing a quietly truncated
			// graph. The root is loaded above and still hard-errors.
			Err((id, e)) => {
				skipped += 1;
				tracing::warn!(target: "kern.persist", kern = %id, error = %e, "skipping corrupt/unreadable kern file");
			}
		}
	}
	if skipped > 0 {
		tracing::warn!(target: "kern.persist", skipped, dir = %dir, "load_dir skipped corrupt kern file(s)");
	}

	let root_kern = kerns
		.get(&root_id)
		.ok_or_else(|| PersistError::MissingNode(root_id.clone()))?
		.clone();
	let quant_mode = read_quant_mode(&quant_dir_sidecar(dir));
	let g = GraphGnn::from_saved_with_mode(
		root_kern,
		network_id,
		dir.to_string(),
		kerns,
		unloaded,
		quant_mode,
	);
	Ok(g)
}

/// The canonical on-disk root kern: the in-memory map entry overlaid with the
/// authoritative root-only fields from `g.root` (id, root_id, purpose, radii,
/// descriptors). Both `save_all` and the tick worker's per-kern persist write
/// the root through this, so a `Persist` task targeting the root can never
/// clobber those fields with a stale map entry.
pub fn merged_root(g: &GraphGnn) -> Kern {
	let root_id = g.root.id.clone();
	let mut merged = g
		.map()
		.get(&root_id)
		.cloned()
		.unwrap_or_else(|| g.root.clone());
	merged.id = g.root.id.clone();
	merged.root_id = g.root.root_id.clone();
	merged.anchor_text = g.root.anchor_text.clone();
	merged.anchor_vec = g.root.anchor_vec.clone();
	merged.inner_radius = g.root.inner_radius;
	merged.outer_radius = g.root.outer_radius;
	// g.root is authoritative for descriptors — REPLACE, don't union. A union (the
	// old `insert` loop) re-added any descriptor still present on the stale map-root
	// base, so a removal on g.root (e.g. `descriptor rm`) never persisted.
	merged.descriptors = g.root.descriptors.clone();
	merged
}

/// Persist a graph's kerns (with the authoritative root overlay) into an explicit
/// store. Shared by [`save_all`] (the graph's own store) and the copy commands
/// (`compress` / `register`) that write into a *different* destination store.
/// Clones the kern map once to apply the `merged_root` overlay; this is the
/// full-persist path (shutdown / explicit save / copy), not the hot per-kern
/// `do_persist`, so the transient clone is fine.
pub fn save_graph_into(
	store: &crate::base::store::Store,
	g: &GraphGnn,
) -> Result<(), crate::base::store::StoreError> {
	let mut kerns = g.map().clone();
	kerns.insert(g.root.id.clone(), merged_root(g));
	store.save_all_kerns(&kerns, &g.network_id, g.quant_mode)
}

/// Persist the whole graph to its own store in one atomic transaction. No-op for
/// an in-memory graph (no store bound). The store's `save_all_kerns` prunes any
/// kern row not in the live set, so a deregistered kern can't resurrect —
/// replacing the old on-disk orphan-file reconcile.
pub fn save_all(g: &GraphGnn) -> Result<(), crate::base::store::StoreError> {
	match g.store() {
		Some(store) => save_graph_into(&store, g),
		None => Ok(()),
	}
}

/// Copy the graph at `src` into a fresh store at `out_dir`, recording
/// `target_mode` as the in-memory index quantization. On-disk vectors are always
/// int8 now (the store's size win), so `target_mode` controls only the HNSW index
/// mode the next load rebuilds with, not the durable vector form.
pub fn compress_dir(
	src: &str,
	out_dir: &str,
	target_mode: QuantizationMode,
) -> Result<(), crate::base::store::StoreError> {
	let mut g = load_dir(src)?;
	g.quant_mode = target_mode;
	let dest = crate::base::store::Store::open(out_dir)?;
	save_graph_into(&dest, &g)
}

#[derive(Serialize, Deserialize)]
struct GraphMeta {
	network_id: String,
}

fn load_network_id(dir: &str) -> String {
	let path = Path::new(dir).join("_meta.kern");
	let data = match fs::read(&path) {
		Ok(d) => d,
		Err(_) => return String::new(),
	};
	match bincode::serde::decode_from_slice::<GraphMeta, _>(&data, bincode_cfg()) {
		Ok((m, _)) => m.network_id,
		Err(_) => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use tempfile::tempdir;

	#[test]
	fn atomic_write_cleans_tmp_and_errors_when_rename_fails() {
		// Force the rename half to fail by making the destination an existing
		// DIRECTORY (renaming a file onto a dir errors on every platform). The
		// tmp file must be cleaned up and the original rename error surfaced.
		let dir = tempdir().unwrap();
		let dst = dir.path().join("target");
		fs::create_dir(&dst).unwrap(); // dst is a directory, not a file

		let err = atomic_write(&dst, b"payload").unwrap_err();
		assert!(matches!(err, PersistError::TmpRename { .. }), "got {err:?}");
		assert!(
			!tmp_path(&dst).exists(),
			"the .tmp file must be cleaned up on rename failure"
		);
	}

	#[test]
	fn atomic_write_then_read_round_trips_on_the_happy_path() {
		let dir = tempdir().unwrap();
		let path = dir.path().join("ok.bin");
		atomic_write(&path, b"hello").expect("write succeeds");
		assert_eq!(fs::read(&path).unwrap(), b"hello");
		assert!(!tmp_path(&path).exists(), "no .tmp left behind on success");
	}

	#[test]
	fn merged_root_overlays_authoritative_fields_over_stale_map_entry() {
		let mut g = GraphGnn::new();
		// Stale root entry in the map: empty purpose/descriptors.
		let mut stale = g.root.clone();
		stale.anchor_text = String::new();
		stale.descriptors.clear();
		g.register(stale);
		// Authoritative values live on g.root.
		g.root.anchor_text = "guiding purpose".to_string();
		g.root
			.descriptors
			.insert("chat".to_string(), "desc".to_string());

		let merged = merged_root(&g);
		assert_eq!(merged.id, g.root.id);
		assert_eq!(merged.anchor_text, "guiding purpose");
		assert_eq!(
			merged.descriptors.get("chat").map(String::as_str),
			Some("desc")
		);
	}

	#[test]
	fn root_persist_via_merged_root_survives_reload() {
		let dir = tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().to_string();
		g.root.anchor_text = "P".to_string();
		g.root.descriptors.insert("k".to_string(), "v".to_string());

		fs::create_dir_all(&g.data_dir).unwrap();
		// This is what do_persist(root) and save_all now write for the root.
		save_kern(&g.data_dir, &merged_root(&g)).unwrap();

		let reloaded = load_kern(&g.data_dir, &g.root.id).unwrap();
		assert_eq!(reloaded.anchor_text, "P");
		assert_eq!(reloaded.descriptors.get("k").map(String::as_str), Some("v"));
	}

	#[test]
	fn named_kern_with_anchor_vec_round_trips() {
		// Guards the bincode-positional assumption behind the purpose->anchor
		// rename: anchor_text/anchor_vec must survive save/load unchanged. If a
		// future edit reorders Kern's fields, the decoded values shift and this
		// fails — catching live-graph corruption before it ships.
		let dir = tempdir().unwrap();
		let d = dir.path().to_string_lossy().to_string();
		let mut k = Kern::new("anchor-work", "root");
		k.anchor_text = "work".to_string();
		k.anchor_vec = vec![0.1, -0.2, 0.3, 0.4];
		k.inner_radius = 0.15;
		k.outer_radius = 0.55;
		save_kern(&d, &k).unwrap();

		let back = load_kern(&d, "anchor-work").unwrap();
		assert_eq!(back.anchor_text, "work");
		assert_eq!(back.anchor_vec, vec![0.1, -0.2, 0.3, 0.4]);
		assert_eq!(back.inner_radius, 0.15);
		assert_eq!(back.outer_radius, 0.55);
		assert!(back.is_named() && back.has_anchor());
	}

	#[test]
	fn load_dir_skips_corrupt_kern_files() {
		let dir = tempdir().unwrap();
		let d = dir.path().to_string_lossy().to_string();
		save_kern(&d, &Kern::new("root", "")).unwrap();
		save_kern(&d, &Kern::new("child1", "root")).unwrap();
		// A corrupt sibling that fails to decode.
		fs::write(format!("{d}/bad.kern"), b"not a valid bincode kern").unwrap();

		let g = load_legacy_dir(&d).expect("load_legacy_dir tolerates a corrupt sibling");
		assert!(g.loaded("child1").is_some(), "valid sibling still loads");
		assert!(
			g.map().keys().all(|k| k != "bad"),
			"corrupt kern is skipped, not inserted"
		);
	}

	#[test]
	fn load_dir_loads_every_sibling() {
		// Parity guard for the parallel decode: a graph with many sibling kerns
		// must come back complete and order-independent. Mixes a corrupt sibling
		// in so the skip path is exercised concurrently with the happy path.
		let dir = tempdir().unwrap();
		let d = dir.path().to_string_lossy().to_string();
		save_kern(&d, &Kern::new("root", "")).unwrap();
		for i in 0..64 {
			save_kern(&d, &Kern::new(format!("child{i}"), "root")).unwrap();
		}
		fs::write(format!("{d}/bad.kern"), b"not a valid bincode kern").unwrap();

		let g = load_legacy_dir(&d).expect("load_legacy_dir loads a large sibling set");
		// root + 64 children, corrupt one skipped.
		assert_eq!(g.map().len(), 65, "root + 64 children all present");
		for i in 0..64 {
			assert!(g.loaded(&format!("child{i}")).is_some(), "child{i} loaded");
		}
		assert!(
			g.map().keys().all(|k| k != "bad"),
			"corrupt sibling skipped"
		);
	}
}
