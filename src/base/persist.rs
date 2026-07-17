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

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
	let mut p = path.as_os_str().to_owned();
	p.push(suffix);
	PathBuf::from(p)
}

// Same dir (same volume) so rename is atomic on both Windows and Unix.
fn tmp_path(path: &Path) -> PathBuf {
	append_suffix(path, ".tmp")
}

// fsync before rename: the crash-atomicity guarantee.
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), PersistError> {
	let tmp = tmp_path(path);
	{
		let mut f = fs::File::create(&tmp)?;
		f.write_all(data)?;
		f.sync_all()?;
	}
	if let Err(source) = fs::rename(&tmp, path) {
		let _ = fs::remove_file(&tmp);
		return Err(PersistError::TmpRename {
			tmp,
			dst: path.to_path_buf(),
			source,
		});
	}
	Ok(())
}

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

// Do NOT extend — new fields use #[serde(default)]. Backfills pre-field shards.
fn backfill_created_at(kern: &mut Kern) {
	let now = std::time::SystemTime::now();
	for t in kern.entities.values_mut() {
		if t.created_at.is_none() {
			t.created_at = Some(now);
		}
	}
}

pub fn load_dir(dir: &str) -> Result<GraphGnn, crate::base::store::StoreError> {
	use crate::base::store::Store;
	use std::sync::Arc;

	let store = Arc::new(Store::open(dir)?);
	graph_from_store(store, dir)
}

// Reload MUST come through here, never load_dir: a same-process double-open of the
// LMDB env can SIGSEGV.
pub fn reload_from_disk(old: &GraphGnn) -> Option<GraphGnn> {
	let store = old.store()?;
	let dir = old.data_dir.clone();
	graph_from_store(store, &dir).ok()
}

fn graph_from_store(
	store: std::sync::Arc<crate::base::store::Store>,
	dir: &str,
) -> Result<GraphGnn, crate::base::store::StoreError> {
	let (mut kerns, mut network_id, quant_mode) = store.load_all_kerns()?;
	if network_id.is_empty() {
		network_id = util::uuid_v4();
	}

	let loaded_epoch = store.read_epoch();
	if !kerns.contains_key("root") {
		let mut g = GraphGnn::new();
		g.data_dir = dir.to_string();
		g.set_store(store);
		g.set_flushed_epoch(loaded_epoch);
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
	g.set_flushed_epoch(loaded_epoch);
	Ok(g)
}

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
	// REPLACE, don't union: a union re-adds descriptors from the stale map base,
	// so a removal on g.root (`descriptor rm`) never persists.
	merged.descriptors = g.root.descriptors.clone();
	merged
}

pub fn save_graph_into(
	store: &crate::base::store::Store,
	g: &GraphGnn,
) -> Result<(), crate::base::store::StoreError> {
	let mut kerns = g.map().clone();
	kerns.insert(g.root.id.clone(), merged_root(g));
	store.save_all_kerns(&kerns, &g.network_id, g.quant_mode)?;
	Ok(())
}

pub fn current_epoch(g: &GraphGnn) -> u64 {
	g.store().map(|s| s.read_epoch()).unwrap_or(0)
}

pub fn flush_guarded(
	g: &GraphGnn,
	expected: u64,
) -> Result<crate::base::store::FlushOutcome, crate::base::store::StoreError> {
	match g.store() {
		Some(store) => {
			let mut kerns = g.map().clone();
			kerns.insert(g.root.id.clone(), merged_root(g));
			store.flush_guarded(&kerns, &g.network_id, g.quant_mode, expected)
		}
		None => Ok(crate::base::store::FlushOutcome::Flushed { epoch: expected }),
	}
}

// Cloned under the read guard so the graph lock can be DROPPED before the flush txn.
pub struct FlushSnapshot {
	store: std::sync::Arc<crate::base::store::Store>,
	kerns: HashMap<String, Kern>,
	network_id: String,
	quant_mode: QuantizationMode,
}

// Call under the read guard; drop it before flush_snapshot runs.
pub fn snapshot_for_flush(g: &GraphGnn) -> Option<FlushSnapshot> {
	let store = g.store()?;
	let mut kerns = g.map().clone();
	kerns.insert(g.root.id.clone(), merged_root(g));
	Some(FlushSnapshot {
		store,
		kerns,
		network_id: g.network_id.clone(),
		quant_mode: g.quant_mode,
	})
}

// No graph lock held. The epoch check runs inside the store write txn, so
// `expected` still holds.
pub fn flush_snapshot(
	snap: &FlushSnapshot,
	expected: u64,
) -> Result<crate::base::store::FlushOutcome, crate::base::store::StoreError> {
	snap
		.store
		.flush_guarded(&snap.kerns, &snap.network_id, snap.quant_mode, expected)
}

// save_all_kerns prunes rows outside the live set, so no kern can resurrect.
pub fn save_all(g: &GraphGnn) -> Result<(), crate::base::store::StoreError> {
	match g.store() {
		Some(store) => save_graph_into(&store, g),
		None => Ok(()),
	}
}

// On-disk vectors are always int8; target_mode sets only the HNSW rebuild mode.
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
		// Renaming a file onto an existing DIRECTORY errors on every platform.
		let dir = tempdir().unwrap();
		let dst = dir.path().join("target");
		fs::create_dir(&dst).unwrap();

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
		let mut stale = g.root.clone();
		stale.anchor_text = String::new();
		stale.descriptors.clear();
		g.register(stale);
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
		save_kern(&g.data_dir, &merged_root(&g)).unwrap();

		let reloaded = load_kern(&g.data_dir, &g.root.id).unwrap();
		assert_eq!(reloaded.anchor_text, "P");
		assert_eq!(reloaded.descriptors.get("k").map(String::as_str), Some("v"));
	}

	#[test]
	fn named_kern_with_anchor_vec_round_trips() {
		// Guards bincode's positional layout: reordering `Kern`'s fields shifts
		// every decoded value and corrupts live graphs.
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
		let dir = tempdir().unwrap();
		let d = dir.path().to_string_lossy().to_string();
		save_kern(&d, &Kern::new("root", "")).unwrap();
		for i in 0..64 {
			save_kern(&d, &Kern::new(format!("child{i}"), "root")).unwrap();
		}
		fs::write(format!("{d}/bad.kern"), b"not a valid bincode kern").unwrap();

		let g = load_legacy_dir(&d).expect("load_legacy_dir loads a large sibling set");
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
