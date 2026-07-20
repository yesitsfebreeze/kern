use std::collections::HashMap;
use std::path::Path;

use heed::types::{Bytes, Str};
use heed::{CompactionOption, Database, Env, EnvOpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::base::types::{Entity, Kern};
use crate::quant::{QuantizationMode, QuantizedVec};

// Headroom is a DURABILITY requirement: a full env fails even the deletes that
// would free space (MDB_MAP_FULL).
const MAP_SIZE: usize = 16 * 1024 * 1024 * 1024;
const MAX_DBS: u32 = 3;

const KERN_DB: &str = "kern";
const COLD_DB: &str = "cold";
const META_DB: &str = "meta";
const META_KEY: &str = "graph";
// Own meta key (not in GraphMeta) so pre-epoch stores read 0.
const EPOCH_KEY: &str = "epoch";

// Version byte prepended ahead of the zstd frame so an old reader rejects a newer
// value instead of mis-decoding it.
const FORMAT_V1: u8 = 1;
// V2 appends StoredKern::temporal; the embedded Kern/Entity layout is unchanged,
// so V1 decodes via StoredKernV1.
const FORMAT_V2: u8 = 2;
// V3 appends Kern::mass; the pre-mass embedded layout decodes via KernPreMass.
const FORMAT_V3: u8 = 3;
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushOutcome {
	Flushed { epoch: u64 },
	RefusedStale { disk_epoch: u64, expected: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
	#[error("io: {0}")]
	Io(#[from] std::io::Error),
	#[error("lmdb: {0}")]
	Lmdb(#[from] heed::Error),
	#[error("bincode encode: {0}")]
	BincodeEncode(#[from] bincode::error::EncodeError),
	#[error("bincode decode: {0}")]
	BincodeDecode(#[from] bincode::error::DecodeError),
	#[error("bad value format version: {0}")]
	BadVersion(u8),
}

// Shared by both backends so encodings never drift; the 1 GiB alloc cap rejects
// corrupt length prefixes (tests/persist_fuzz.rs).
pub(crate) fn bincode_cfg() -> impl bincode::config::Config {
	bincode::config::standard().with_limit::<{ 1024 * 1024 * 1024 }>()
}

// [FORMAT_V3] ++ zstd(bincode(v)).
fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>, StoreError> {
	let raw = bincode::serde::encode_to_vec(v, bincode_cfg())?;
	let comp = zstd::encode_all(raw.as_slice(), ZSTD_LEVEL)?;
	let mut out = Vec::with_capacity(comp.len() + 1);
	out.push(FORMAT_V3);
	out.extend_from_slice(&comp);
	Ok(out)
}

fn strip_version(bytes: &[u8]) -> Result<(u8, Vec<u8>), StoreError> {
	let (&ver, body) = bytes.split_first().ok_or(StoreError::BadVersion(0))?;
	if !(FORMAT_V1..=FORMAT_V3).contains(&ver) {
		return Err(StoreError::BadVersion(ver));
	}
	Ok((ver, zstd::decode_all(body)?))
}

// For values whose layout is identical across V1/V2; StoredKern must use
// decode_stored_kern instead.
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
	let (_ver, raw) = strip_version(bytes)?;
	let (v, _) = bincode::serde::decode_from_slice(&raw, bincode_cfg())?;
	Ok(v)
}

fn decode_stored_kern(bytes: &[u8]) -> Result<StoredKern, StoreError> {
	let (ver, raw) = strip_version(bytes)?;
	match ver {
		FORMAT_V3 => Ok(bincode::serde::decode_from_slice(&raw, bincode_cfg())?.0),
		FORMAT_V2 => {
			let (v2, _): (StoredKernV2, _) = bincode::serde::decode_from_slice(&raw, bincode_cfg())?;
			Ok(v2.into())
		}
		// FORMAT_V1 (validated by strip_version): pre-temporal, pre-mass layout.
		_ => {
			let (v1, _): (StoredKernV1, _) = bincode::serde::decode_from_slice(&raw, bincode_cfg())?;
			Ok(v1.into())
		}
	}
}

// Do NOT persist QuantizedVec directly — its skip_serializing_if desyncs
// positional bincode. Every field here is always present.
#[derive(Serialize, Deserialize)]
pub struct StoredVec {
	pub scale: f32,
	pub q: Vec<i8>,
}

impl StoredVec {
	fn encode(v: &[f32]) -> Self {
		let qv = QuantizedVec::encode(v, QuantizationMode::Int8);
		StoredVec {
			scale: qv.scale,
			q: qv.q,
		}
	}

	fn decode(&self) -> Vec<f32> {
		self.q.iter().map(|&x| (x as f32) * self.scale).collect()
	}
}

// Entity's temporal fields are serde(skip) for byte-stable layout, so the store
// carries them here.
#[derive(Serialize, Deserialize, Default)]
pub struct StoredTemporal {
	pub valid_from: Option<std::time::SystemTime>,
	pub valid_to: Option<std::time::SystemTime>,
	pub invalidated_at: Option<std::time::SystemTime>,
}

impl StoredTemporal {
	fn is_set(e: &Entity) -> bool {
		e.valid_from.is_some() || e.valid_to.is_some() || e.invalidated_at.is_some()
	}
}

#[derive(Serialize, Deserialize)]
pub struct StoredKern {
	pub kern: Kern,
	pub entity_vecs: HashMap<String, StoredVec>,
	pub reason_vecs: HashMap<String, StoredVec>,
	// Appended field — new in FORMAT_V2.
	pub temporal: HashMap<String, StoredTemporal>,
}

impl StoredKern {
	pub fn from_kern(k: &Kern) -> Self {
		let mut kern = k.clone();
		let mut entity_vecs = HashMap::new();
		let mut reason_vecs = HashMap::new();
		let mut temporal = HashMap::new();
		for (id, e) in kern.entities.iter_mut() {
			if !e.vector.is_empty() {
				entity_vecs.insert(id.clone(), StoredVec::encode(&e.vector));
			}
			if StoredTemporal::is_set(e) {
				temporal.insert(
					id.clone(),
					StoredTemporal {
						valid_from: e.valid_from,
						valid_to: e.valid_to,
						invalidated_at: e.invalidated_at,
					},
				);
			}
			// vector is restored from the side-map on load; gnn_vector is recomputed,
			// never persisted.
			e.vector = Vec::new();
			e.gnn_vector = Vec::new();
		}
		for (id, r) in kern.reasons.iter_mut() {
			if !r.vector.is_empty() {
				reason_vecs.insert(id.clone(), StoredVec::encode(&r.vector));
			}
			r.vector = Vec::new();
		}
		StoredKern {
			kern,
			entity_vecs,
			reason_vecs,
			temporal,
		}
	}

	pub fn into_kern(self) -> Kern {
		let mut kern = self.kern;
		for (id, e) in kern.entities.iter_mut() {
			if let Some(q) = self.entity_vecs.get(id) {
				e.vector = q.decode();
			}
			if let Some(t) = self.temporal.get(id) {
				e.valid_from = t.valid_from;
				e.valid_to = t.valid_to;
				e.invalidated_at = t.invalidated_at;
			}
		}
		for (id, r) in kern.reasons.iter_mut() {
			if let Some(q) = self.reason_vecs.get(id) {
				r.vector = q.decode();
			}
		}
		kern
	}
}

// FORMAT_V2 mirror: pre-mass Kern with the temporal side-map.
#[derive(Serialize, Deserialize)]
struct StoredKernV2 {
	kern: crate::base::types::KernPreMass,
	entity_vecs: HashMap<String, StoredVec>,
	reason_vecs: HashMap<String, StoredVec>,
	temporal: HashMap<String, StoredTemporal>,
}

impl From<StoredKernV2> for StoredKern {
	fn from(v2: StoredKernV2) -> Self {
		StoredKern {
			kern: v2.kern.into(),
			entity_vecs: v2.entity_vecs,
			reason_vecs: v2.reason_vecs,
			temporal: v2.temporal,
		}
	}
}

// FORMAT_V1 mirror without the temporal side-map; the embedded entity bytes are
// identical across versions.
#[derive(Serialize, Deserialize)]
struct StoredKernV1 {
	kern: crate::base::types::KernPreMass,
	entity_vecs: HashMap<String, StoredVec>,
	reason_vecs: HashMap<String, StoredVec>,
}

impl From<StoredKernV1> for StoredKern {
	fn from(v1: StoredKernV1) -> Self {
		StoredKern {
			kern: v1.kern.into(),
			entity_vecs: v1.entity_vecs,
			reason_vecs: v1.reason_vecs,
			temporal: HashMap::new(),
		}
	}
}

#[derive(Serialize, Deserialize)]
struct GraphMeta {
	network_id: String,
	quant_mode: QuantizationMode,
}

// One embedded LMDB env per data_dir; LMDB gives many-reader / single-writer
// concurrency across processes.
pub struct Store {
	env: Env,
	kern: Database<Str, Bytes>,
	cold: Database<Str, Bytes>,
	meta: Database<Str, Bytes>,
	dir: std::path::PathBuf,
}

impl Store {
	// All named databases are created up front so later read txns never miss one.
	pub fn open(dir: &str) -> Result<Self, StoreError> {
		std::fs::create_dir_all(dir)?;
		let path = Path::new(dir);
		// SAFETY: mmap-ing a file is unsafe iff another process truncates/corrupts
		// it underneath us. The data dir is kern-owned; the only writers are kern
		// processes, which coordinate through LMDB's own lock. No external truncation.
		let env = unsafe {
			EnvOpenOptions::new()
				.map_size(MAP_SIZE)
				.max_dbs(MAX_DBS)
				.open(path)?
		};
		let mut wtxn = env.write_txn()?;
		let kern = env.create_database::<Str, Bytes>(&mut wtxn, Some(KERN_DB))?;
		let cold = env.create_database::<Str, Bytes>(&mut wtxn, Some(COLD_DB))?;
		let meta = env.create_database::<Str, Bytes>(&mut wtxn, Some(META_DB))?;
		wtxn.commit()?;
		Ok(Self {
			env,
			kern,
			cold,
			meta,
			dir: path.to_path_buf(),
		})
	}

	pub fn data_file_len(&self) -> u64 {
		std::fs::metadata(self.dir.join("data.mdb"))
			.map(|m| m.len())
			.unwrap_or(0)
	}

	fn put<T: Serialize>(
		&self,
		db: Database<Str, Bytes>,
		key: &str,
		value: &T,
	) -> Result<(), StoreError> {
		let bytes = encode(value)?;
		let mut wtxn = self.env.write_txn()?;
		db.put(&mut wtxn, key, &bytes)?;
		wtxn.commit()?;
		Ok(())
	}

	fn get<T: DeserializeOwned>(
		&self,
		db: Database<Str, Bytes>,
		key: &str,
	) -> Result<Option<T>, StoreError> {
		self.get_with(db, key, decode)
	}

	fn get_with<T>(
		&self,
		db: Database<Str, Bytes>,
		key: &str,
		decode_fn: impl Fn(&[u8]) -> Result<T, StoreError>,
	) -> Result<Option<T>, StoreError> {
		let rtxn = self.env.read_txn()?;
		match db.get(&rtxn, key)? {
			Some(b) => Ok(Some(decode_fn(b)?)),
			None => Ok(None),
		}
	}

	fn remove(&self, db: Database<Str, Bytes>, key: &str) -> Result<(), StoreError> {
		let mut wtxn = self.env.write_txn()?;
		db.delete(&mut wtxn, key)?;
		wtxn.commit()?;
		Ok(())
	}

	fn scan<T: DeserializeOwned>(
		&self,
		db: Database<Str, Bytes>,
	) -> Result<Vec<(String, T)>, StoreError> {
		self.scan_with(db, decode)
	}

	fn scan_with<T>(
		&self,
		db: Database<Str, Bytes>,
		decode_fn: impl Fn(&[u8]) -> Result<T, StoreError>,
	) -> Result<Vec<(String, T)>, StoreError> {
		let rtxn = self.env.read_txn()?;
		let mut out = Vec::new();
		for item in db.iter(&rtxn)? {
			let (k, v) = item?;
			match decode_fn(v) {
				Ok(val) => out.push((k.to_string(), val)),
				Err(e) => {
					tracing::warn!(target: "kern.store", key = %k, error = %e, "skipping corrupt value");
				}
			}
		}
		Ok(out)
	}

	// Missing/garbled reads as 0 — the epoch is advisory and must never fail a load.
	pub fn read_epoch(&self) -> u64 {
		self
			.get::<u64>(self.meta, EPOCH_KEY)
			.ok()
			.flatten()
			.unwrap_or(0)
	}

	// Read inside the open txn so the guard's check and the write commit atomically.
	fn epoch_in(&self, wtxn: &heed::RwTxn) -> Result<u64, StoreError> {
		match self.meta.get(wtxn, EPOCH_KEY)? {
			Some(b) => Ok(decode::<u64>(b).unwrap_or(0)),
			None => Ok(0),
		}
	}

	// Destructive prune-and-write shared by both save paths; stamps next_epoch.
	fn write_snapshot(
		&self,
		wtxn: &mut heed::RwTxn,
		kerns: &HashMap<String, Kern>,
		network_id: &str,
		quant_mode: QuantizationMode,
		next_epoch: u64,
	) -> Result<(), StoreError> {
		// Collect existing keys first (immutable borrow of the txn), then mutate —
		// can't hold the iterator borrow across put/delete.
		let existing: Vec<String> = {
			let mut v = Vec::new();
			for item in self.kern.iter(wtxn)? {
				let (k, _) = item?;
				v.push(k.to_string());
			}
			v
		};
		for id in existing {
			if !kerns.contains_key(&id) {
				self.kern.delete(wtxn, id.as_str())?;
			}
		}
		for (id, kern) in kerns {
			let bytes = encode(&StoredKern::from_kern(kern))?;
			self.kern.put(wtxn, id.as_str(), &bytes)?;
		}
		let meta = GraphMeta {
			network_id: network_id.to_string(),
			quant_mode,
		};
		let meta_bytes = encode(&meta)?;
		self.meta.put(wtxn, META_KEY, &meta_bytes)?;
		let epoch_bytes = encode(&next_epoch)?;
		self.meta.put(wtxn, EPOCH_KEY, &epoch_bytes)?;
		Ok(())
	}

	// One atomic commit — a crash leaves old or new, never a torn mix.
	pub fn save_all_kerns(
		&self,
		kerns: &HashMap<String, Kern>,
		network_id: &str,
		quant_mode: QuantizationMode,
	) -> Result<u64, StoreError> {
		let mut wtxn = self.env.write_txn()?;
		let next = self.epoch_in(&wtxn)?.wrapping_add(1);
		self.write_snapshot(&mut wtxn, kerns, network_id, quant_mode, next)?;
		wtxn.commit()?;
		Ok(next)
	}

	// Writes ONLY while the on-disk epoch still equals `expected`, else refuses
	// untouched; check and write share one txn.
	pub fn flush_guarded(
		&self,
		kerns: &HashMap<String, Kern>,
		network_id: &str,
		quant_mode: QuantizationMode,
		expected: u64,
	) -> Result<FlushOutcome, StoreError> {
		let mut wtxn = self.env.write_txn()?;
		let disk = self.epoch_in(&wtxn)?;
		if disk > expected {
			wtxn.abort();
			return Ok(FlushOutcome::RefusedStale {
				disk_epoch: disk,
				expected,
			});
		}
		let next = disk.wrapping_add(1);
		self.write_snapshot(&mut wtxn, kerns, network_id, quant_mode, next)?;
		wtxn.commit()?;
		Ok(FlushOutcome::Flushed { epoch: next })
	}

	pub fn load_all_kerns(
		&self,
	) -> Result<(HashMap<String, Kern>, String, QuantizationMode), StoreError> {
		let stored: Vec<(String, StoredKern)> = self.scan_with(self.kern, decode_stored_kern)?;
		let mut kerns = HashMap::with_capacity(stored.len());
		for (id, sk) in stored {
			kerns.insert(id, sk.into_kern());
		}
		let (network_id, quant_mode) = match self.get::<GraphMeta>(self.meta, META_KEY)? {
			Some(m) => (m.network_id, m.quant_mode),
			None => (String::new(), QuantizationMode::None),
		};
		Ok((kerns, network_id, quant_mode))
	}

	pub fn save_one_kern(&self, kern: &Kern) -> Result<(), StoreError> {
		self.put(self.kern, &kern.id, &StoredKern::from_kern(kern))
	}

	pub fn load_one_kern(&self, id: &str) -> Result<Option<Kern>, StoreError> {
		Ok(
			self
				.get_with(self.kern, id, decode_stored_kern)?
				.map(StoredKern::into_kern),
		)
	}

	pub fn delete_one_kern(&self, id: &str) -> Result<(), StoreError> {
		self.remove(self.kern, id)
	}

	pub fn cold_spill(&self, entity: &Entity) -> Result<(), StoreError> {
		self.put(self.cold, &entity.id, entity)?;
		self.cold_cap(crate::base::constants::COLD_MAX_ENTRIES)?;
		Ok(())
	}

	pub fn cold_get(&self, id: &str) -> Result<Option<Entity>, StoreError> {
		self.get(self.cold, id)
	}

	pub fn cold_all(&self) -> Result<Vec<Entity>, StoreError> {
		Ok(self.scan(self.cold)?.into_iter().map(|(_, e)| e).collect())
	}

	pub fn cold_put_all(&self, entities: &[Entity]) -> Result<(), StoreError> {
		let mut wtxn = self.env.write_txn()?;
		for e in entities {
			let bytes = encode(e)?;
			self.cold.put(&mut wtxn, &e.id, &bytes)?;
		}
		wtxn.commit()?;
		self.cold_cap(crate::base::constants::COLD_MAX_ENTRIES)?;
		Ok(())
	}

	pub fn cold_search(&self, query_vec: &[f32], k: usize) -> Result<Vec<(Entity, f64)>, StoreError> {
		if query_vec.is_empty() || k == 0 {
			return Ok(Vec::new());
		}
		let rows: Vec<(String, Entity)> = self.scan(self.cold)?;
		let mut scored: Vec<(Entity, f64)> = rows
			.into_iter()
			.filter_map(|(_, e)| {
				if e.vector.len() != query_vec.len() {
					return None;
				}
				let s = crate::base::math::cosine(query_vec, &e.vector);
				if s.is_finite() {
					Some((e, s))
				} else {
					None
				}
			})
			.collect();
		// Ties broken by id ascending so truncation is deterministic — LMDB scan
		// order must not decide which equal-cosine entities survive.
		scored.sort_by(|a, b| crate::base::util::cmp_rank(a.1, &a.0.id, b.1, &b.0.id));
		scored.truncate(k);
		Ok(scored)
	}

	fn cold_cap(&self, max: usize) -> Result<(), StoreError> {
		let len = {
			let rtxn = self.env.read_txn()?;
			self.cold.len(&rtxn)? as usize
		};
		if len <= max {
			return Ok(());
		}
		let mut rows: Vec<(String, Entity)> = self.scan(self.cold)?;
		rows.sort_by_key(|r| std::cmp::Reverse(r.1.created_at));
		let drop_ids: Vec<String> = rows.into_iter().skip(max).map(|(id, _)| id).collect();
		let mut wtxn = self.env.write_txn()?;
		for id in &drop_ids {
			self.cold.delete(&mut wtxn, id.as_str())?;
		}
		wtxn.commit()?;
		Ok(())
	}
}

fn compact_tmp(dir: &Path) -> std::path::PathBuf {
	dir.join("data.mdb.compact")
}

// Caller MUST drop every env handle first; retries ride out Windows' async unmap lag.
fn swap_compacted(dir: &str) -> Result<(u64, u64), StoreError> {
	let path = Path::new(dir);
	let data = path.join("data.mdb");
	let tmp = compact_tmp(path);
	let old_len = std::fs::metadata(&data).map(|m| m.len()).unwrap_or(0);

	let mut last_err = None;
	for attempt in 0..25 {
		match std::fs::rename(&tmp, &data) {
			Ok(()) => {
				let _ = std::fs::remove_file(path.join("lock.mdb"));
				let new_len = std::fs::metadata(&data).map(|m| m.len()).unwrap_or(0);
				return Ok((old_len, new_len));
			}
			Err(e) => {
				last_err = Some(e);
				std::thread::sleep(std::time::Duration::from_millis(100 + attempt * 4));
			}
		}
	}
	let _ = std::fs::remove_file(&tmp);
	Err(StoreError::Io(last_err.unwrap_or_else(|| {
		std::io::Error::other("compaction swap failed")
	})))
}

// The only way to shrink LMDB's high-water mark. REQUIRES exclusive access:
// run offline, daemon stopped.
pub fn compact_dir(dir: &str) -> Result<(u64, u64), StoreError> {
	let path = Path::new(dir);
	// A full env copy costs more than the space it could reclaim below this size.
	let old_len = std::fs::metadata(path.join("data.mdb"))
		.map(|m| m.len())
		.unwrap_or(0);
	if old_len < crate::base::constants::COLD_COMPACT_MIN_BYTES {
		return Ok((old_len, old_len));
	}

	let tmp = compact_tmp(path);
	let _ = std::fs::remove_file(&tmp);

	{
		let env = unsafe {
			EnvOpenOptions::new()
				.map_size(MAP_SIZE)
				.max_dbs(MAX_DBS)
				.open(path)?
		};
		env.copy_to_file(&tmp, CompactionOption::Enabled)?;
		// Block until the env is truly closed (mmap released) before the swap.
		env.prepare_for_closing().wait();
	}

	swap_compacted(dir)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{mk_entity, EntityKind};
	use std::time::{Duration, UNIX_EPOCH};

	#[derive(Debug, PartialEq, Serialize, Deserialize)]
	struct Sample {
		name: String,
		nums: Vec<f64>,
	}

	fn tmp() -> tempfile::TempDir {
		tempfile::tempdir().unwrap()
	}

	fn dir_of(d: &tempfile::TempDir) -> String {
		d.path().to_string_lossy().to_string()
	}

	#[test]
	fn codec_roundtrips_a_struct() {
		let v = Sample {
			name: "hello".into(),
			nums: vec![1.0, -2.5, 3.25],
		};
		let bytes = encode(&v).unwrap();
		let back: Sample = decode(&bytes).unwrap();
		assert_eq!(v, back);
	}

	#[test]
	fn codec_prepends_format_version() {
		let bytes = encode(&Sample {
			name: "x".into(),
			nums: vec![],
		})
		.unwrap();
		assert_eq!(
			bytes[0], FORMAT_V3,
			"first byte is the current write version"
		);
	}

	#[test]
	fn decode_accepts_both_v1_and_v2_for_layout_invariant_values() {
		let mut bytes = encode(&Sample {
			name: "x".into(),
			nums: vec![1.0],
		})
		.unwrap();
		let want = Sample {
			name: "x".into(),
			nums: vec![1.0],
		};
		assert_eq!(decode::<Sample>(&bytes).unwrap(), want, "V2 decodes");
		bytes[0] = FORMAT_V1;
		assert_eq!(
			decode::<Sample>(&bytes).unwrap(),
			want,
			"V1 tag still decodes"
		);
	}

	#[test]
	fn decode_rejects_unknown_version() {
		let mut bytes = encode(&Sample {
			name: "x".into(),
			nums: vec![1.0],
		})
		.unwrap();
		bytes[0] = 0xFF;
		match decode::<Sample>(&bytes) {
			Err(StoreError::BadVersion(0xFF)) => {}
			other => panic!("expected BadVersion(0xFF), got {other:?}"),
		}
	}

	#[test]
	fn put_get_remove_roundtrip() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let v = Sample {
			name: "k".into(),
			nums: vec![0.1, 0.2],
		};
		s.put(s.kern, "k", &v).unwrap();
		assert_eq!(s.get::<Sample>(s.kern, "k").unwrap(), Some(v));
		s.remove(s.kern, "k").unwrap();
		assert_eq!(s.get::<Sample>(s.kern, "k").unwrap(), None);
	}

	#[test]
	fn get_absent_is_none() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		assert_eq!(s.get::<Sample>(s.kern, "missing").unwrap(), None);
	}

	#[test]
	fn scan_returns_all_rows() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		for i in 0..5 {
			s.put(
				s.kern,
				&format!("k{i}"),
				&Sample {
					name: format!("n{i}"),
					nums: vec![i as f64],
				},
			)
			.unwrap();
		}
		let mut rows: Vec<(String, Sample)> = s.scan(s.kern).unwrap();
		rows.sort_by(|a, b| a.0.cmp(&b.0));
		assert_eq!(rows.len(), 5);
		assert_eq!(rows[2].0, "k2");
		assert_eq!(rows[2].1.name, "n2");
	}

	#[test]
	fn reopen_persists_data() {
		let d = tmp();
		let dir = dir_of(&d);
		{
			let s = Store::open(&dir).unwrap();
			s.put(
				s.kern,
				"k",
				&Sample {
					name: "durable".into(),
					nums: vec![9.0],
				},
			)
			.unwrap();
		}
		let s2 = Store::open(&dir).unwrap();
		assert_eq!(
			s2.get::<Sample>(s2.kern, "k").unwrap().unwrap().name,
			"durable"
		);
	}

	fn kern_with(id: &str, entity: Entity) -> Kern {
		let mut k = Kern::new(id, "");
		k.entities.insert(entity.id.clone(), entity);
		k
	}

	#[test]
	fn stored_kern_roundtrip_quantizes_and_drops_gnn() {
		let mut e = mk_entity("e1", "a fact", 1.0, EntityKind::Claim);
		e.vector = vec![0.1, -0.2, 0.3, 0.4];
		e.gnn_vector = vec![1.0, 1.0, 1.0, 1.0];
		let k = kern_with("k", e);

		let back = StoredKern::from_kern(&k).into_kern();
		let be = &back.entities["e1"];
		assert_eq!(be.vector.len(), 4, "vector recovered");
		for (got, want) in be.vector.iter().zip([0.1, -0.2, 0.3, 0.4]) {
			assert!(
				(got - want).abs() < 0.02,
				"int8 within tolerance: {got} vs {want}"
			);
		}
		assert!(
			be.gnn_vector.is_empty(),
			"gnn_vector is dropped, not persisted"
		);
		assert_eq!(be.heat, 1.0, "non-vector fields survive");
		assert_eq!(be.text(), "a fact");
	}

	#[test]
	fn stored_kern_handles_empty_vectors() {
		let e = mk_entity("e1", "novec", 0.0, EntityKind::Claim);
		let mut e = e;
		e.vector = Vec::new();
		let k = kern_with("k", e);
		let sk = StoredKern::from_kern(&k);
		assert!(
			sk.entity_vecs.is_empty(),
			"no side-map entry for an empty vector"
		);
		let back = sk.into_kern();
		assert!(!back.entities["e1"].has_vector());
	}

	#[test]
	fn stored_kern_v2_roundtrips_temporal_stamps() {
		let t0 = UNIX_EPOCH + Duration::from_secs(1000);
		let t1 = UNIX_EPOCH + Duration::from_secs(2000);
		let mut e = mk_entity("e1", "a claim", 1.0, EntityKind::Claim);
		e.vector = vec![0.1, 0.2];
		e.valid_from = Some(t0);
		e.valid_to = Some(t1);
		e.invalidated_at = Some(t1);
		let k = kern_with("k", e);

		let bytes = encode(&StoredKern::from_kern(&k)).unwrap();
		assert_eq!(bytes[0], FORMAT_V3, "kern rows are written as V3");
		let back = decode_stored_kern(&bytes).unwrap().into_kern();
		let be = &back.entities["e1"];
		assert_eq!(be.valid_from, Some(t0), "valid_from survives");
		assert_eq!(be.valid_to, Some(t1), "valid_to survives");
		assert_eq!(be.invalidated_at, Some(t1), "invalidated_at survives");
	}

	#[test]
	fn stored_kern_v1_blob_decodes_with_none_temporal() {
		let mut e = mk_entity("e1", "old claim", 1.0, EntityKind::Fact);
		e.vector = vec![0.3, 0.4];
		let k = kern_with("k", e);

		let sk = StoredKern::from_kern(&k);
		let v1 = StoredKernV1 {
			kern: sk.kern.into(),
			entity_vecs: sk.entity_vecs,
			reason_vecs: sk.reason_vecs,
		};
		let raw = bincode::serde::encode_to_vec(&v1, bincode_cfg()).unwrap();
		let comp = zstd::encode_all(raw.as_slice(), ZSTD_LEVEL).unwrap();
		let mut bytes = vec![FORMAT_V1];
		bytes.extend_from_slice(&comp);

		let back = decode_stored_kern(&bytes).unwrap().into_kern();
		let be = &back.entities["e1"];
		assert_eq!(
			be.text(),
			"old claim",
			"entity content intact across V1 decode"
		);
		assert!((be.vector[0] - 0.3).abs() < 0.02, "vector recovered");
		assert_eq!(be.valid_from, None, "V1 entity has no valid_from");
		assert_eq!(be.valid_to, None, "V1 entity has no valid_to");
		assert_eq!(be.invalidated_at, None, "V1 entity is not invalidated");
	}

	#[test]
	fn v1_kern_row_loads_through_the_store_not_skipped() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let k = kern_with("k", mk_entity("e1", "legacy", 1.0, EntityKind::Claim));
		let sk = StoredKern::from_kern(&k);
		let v1 = StoredKernV1 {
			kern: sk.kern.into(),
			entity_vecs: sk.entity_vecs,
			reason_vecs: sk.reason_vecs,
		};
		let raw = bincode::serde::encode_to_vec(&v1, bincode_cfg()).unwrap();
		let comp = zstd::encode_all(raw.as_slice(), ZSTD_LEVEL).unwrap();
		let mut bytes = vec![FORMAT_V1];
		bytes.extend_from_slice(&comp);
		{
			let mut wtxn = s.env.write_txn().unwrap();
			s.kern.put(&mut wtxn, "k", bytes.as_slice()).unwrap();
			wtxn.commit().unwrap();
		}

		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(
			loaded.contains_key("k"),
			"V1 row loaded, not skipped as corrupt"
		);
		assert_eq!(loaded["k"].entities["e1"].text(), "legacy");
	}

	#[test]
	fn save_then_load_graph_roundtrip() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let mut e = mk_entity("e1", "hello", 2.0, EntityKind::Fact);
		e.vector = vec![0.5, -0.5, 0.25];
		let mut kerns = HashMap::new();
		kerns.insert("root".to_string(), Kern::new("root", ""));
		kerns.insert("k".to_string(), kern_with("k", e));

		s.save_all_kerns(&kerns, "net-123", QuantizationMode::Int8)
			.unwrap();
		let (loaded, net, qm) = s.load_all_kerns().unwrap();

		assert_eq!(net, "net-123");
		assert_eq!(qm, QuantizationMode::Int8);
		assert_eq!(loaded.len(), 2);
		let be = &loaded["k"].entities["e1"];
		assert_eq!(be.text(), "hello");
		assert!((be.vector[0] - 0.5).abs() < 0.02);
	}

	#[test]
	fn epoch_starts_at_zero_and_every_full_save_bumps_it() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		assert_eq!(s.read_epoch(), 0, "a fresh store reads epoch 0");
		let mut kerns = HashMap::new();
		kerns.insert("root".to_string(), Kern::new("root", ""));
		assert_eq!(
			s.save_all_kerns(&kerns, "n", QuantizationMode::None)
				.unwrap(),
			1,
			"first save bumps to 1"
		);
		assert_eq!(s.read_epoch(), 1);
		assert_eq!(
			s.save_all_kerns(&kerns, "n", QuantizationMode::None)
				.unwrap(),
			2,
			"each save advances the epoch"
		);
		assert_eq!(s.read_epoch(), 2);
	}

	#[test]
	fn flush_guarded_refuses_a_stale_smaller_snapshot_and_keeps_disk_rows() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();

		let mut a = HashMap::new();
		a.insert("root".to_string(), Kern::new("root", ""));
		a.insert(
			"ka".to_string(),
			kern_with("ka", mk_entity("ea", "durable", 1.0, EntityKind::Claim)),
		);
		assert_eq!(
			s.flush_guarded(&a, "n", QuantizationMode::None, 0).unwrap(),
			FlushOutcome::Flushed { epoch: 1 },
		);

		let mut b = HashMap::new();
		b.insert("root".to_string(), Kern::new("root", ""));
		assert_eq!(
			s.flush_guarded(&b, "n", QuantizationMode::None, 0).unwrap(),
			FlushOutcome::RefusedStale {
				disk_epoch: 1,
				expected: 0,
			},
		);
		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(
			loaded.contains_key("ka"),
			"a refused stale flush must not drop the row another writer committed"
		);
		assert_eq!(s.read_epoch(), 1, "a refusal does not advance the epoch");

		assert_eq!(
			s.flush_guarded(&b, "n", QuantizationMode::None, 1).unwrap(),
			FlushOutcome::Flushed { epoch: 2 },
		);
		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(
			!loaded.contains_key("ka"),
			"an up-to-date flush still prunes removed rows"
		);
	}

	#[test]
	fn save_all_prunes_removed_kerns() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let mut kerns = HashMap::new();
		kerns.insert("a".to_string(), Kern::new("a", ""));
		kerns.insert("b".to_string(), Kern::new("b", ""));
		s.save_all_kerns(&kerns, "n", QuantizationMode::None)
			.unwrap();

		kerns.remove("b");
		s.save_all_kerns(&kerns, "n", QuantizationMode::None)
			.unwrap();

		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(loaded.contains_key("a"));
		assert!(!loaded.contains_key("b"), "removed kern pruned from disk");
	}

	#[test]
	fn compact_dir_skips_a_store_below_the_min_size() {
		let d = tmp();
		let dir = dir_of(&d);
		{
			let s = Store::open(&dir).unwrap();
			let mut kerns = HashMap::new();
			kerns.insert("root".to_string(), Kern::new("root", ""));
			s.save_all_kerns(&kerns, "n", QuantizationMode::Int8)
				.unwrap();
			assert!(
				s.data_file_len() < crate::base::constants::COLD_COMPACT_MIN_BYTES,
				"tiny store"
			);
		}

		let (old_len, new_len) = compact_dir(&dir).unwrap();
		assert_eq!(
			old_len, new_len,
			"under the threshold compaction is a no-op"
		);
		assert!(
			!compact_tmp(Path::new(&dir)).exists(),
			"no env copy was made"
		);

		let s2 = Store::open(&dir).unwrap();
		assert!(s2.load_all_kerns().unwrap().0.contains_key("root"));
	}

	#[test]
	fn compact_dir_shrinks_after_bulk_delete() {
		let d = tmp();
		let dir = dir_of(&d);
		{
			let s = Store::open(&dir).unwrap();
			let mut kerns = HashMap::new();
			kerns.insert("root".to_string(), Kern::new("root", ""));
			for i in 0..2000 {
				let mut e = mk_entity(&format!("e{i}"), &"x".repeat(512), 1.0, EntityKind::Claim);
				e.vector = (0..256).map(|j| ((i + j) as f64).sin() as f32).collect();
				kerns.insert(format!("k{i}"), kern_with(&format!("k{i}"), e));
			}
			s.save_all_kerns(&kerns, "n", QuantizationMode::Int8)
				.unwrap();
			let bloated = s.data_file_len();

			let mut small = HashMap::new();
			small.insert("root".to_string(), Kern::new("root", ""));
			small.insert("k0".to_string(), kerns.remove("k0").unwrap());
			s.save_all_kerns(&small, "n", QuantizationMode::Int8)
				.unwrap();
			assert!(
				s.data_file_len() >= bloated * 9 / 10,
				"LMDB keeps ~all of the high-water mark after delete: {} vs peak {bloated}",
				s.data_file_len(),
			);
		} // env dropped so the offline compactor can swap the file

		let (old_len, new_len) = compact_dir(&dir).unwrap();
		assert!(
			new_len < old_len,
			"compaction shrinks the file: {old_len} -> {new_len}"
		);

		let s2 = Store::open(&dir).unwrap();
		let (loaded, _, _) = s2.load_all_kerns().unwrap();
		assert!(loaded.contains_key("root"), "root survives compaction");
		assert!(loaded.contains_key("k0"), "survivor survives compaction");
		assert_eq!(
			loaded.len(),
			2,
			"only the live rows remain after compaction"
		);
	}

	#[test]
	fn single_kern_save_load_delete() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let k = kern_with("k", mk_entity("e1", "x", 0.0, EntityKind::Claim));
		s.save_one_kern(&k).unwrap();
		assert!(s.load_one_kern("k").unwrap().is_some());
		s.delete_one_kern("k").unwrap();
		assert!(s.load_one_kern("k").unwrap().is_none());
		s.delete_one_kern("k").unwrap();
	}

	#[test]
	fn corrupt_kern_value_is_skipped() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		s.save_one_kern(&kern_with(
			"good",
			mk_entity("e", "ok", 0.0, EntityKind::Claim),
		))
		.unwrap();
		{
			let mut wtxn = s.env.write_txn().unwrap();
			s.kern
				.put(&mut wtxn, "bad", b"not a valid value".as_slice())
				.unwrap();
			wtxn.commit().unwrap();
		}
		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(loaded.contains_key("good"), "valid kern loads");
		assert!(
			!loaded.contains_key("bad"),
			"corrupt kern skipped, not fatal"
		);
	}

	#[test]
	fn cold_spill_then_get() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let e = mk_entity("a", "hello cold", 0.0, EntityKind::Claim);
		s.cold_spill(&e).unwrap();
		let got = s.cold_get("a").unwrap().unwrap();
		assert_eq!(got.text(), "hello cold");
	}

	#[test]
	fn cold_latest_wins() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		s.cold_spill(&mk_entity("x", "v1", 1.0, EntityKind::Claim))
			.unwrap();
		s.cold_spill(&mk_entity("x", "v2", 5.0, EntityKind::Claim))
			.unwrap();
		let got = s.cold_get("x").unwrap().unwrap();
		assert_eq!(got.heat, 5.0, "a put overwrites — latest wins, no dup rows");
	}

	#[test]
	fn cold_search_ranks_by_cosine() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let mut ex = mk_entity("ex", "x axis", 0.0, EntityKind::Claim);
		ex.vector = vec![1.0, 0.0];
		let mut ey = mk_entity("ey", "y axis", 0.0, EntityKind::Claim);
		ey.vector = vec![0.0, 1.0];
		let mut enear = mk_entity("enear", "near x", 0.0, EntityKind::Claim);
		enear.vector = vec![0.9, 0.1];
		s.cold_spill(&ex).unwrap();
		s.cold_spill(&ey).unwrap();
		s.cold_spill(&enear).unwrap();

		let hits = s.cold_search(&[1.0, 0.0], 2).unwrap();
		assert_eq!(hits.len(), 2);
		assert_eq!(hits[0].0.id, "ex", "closest to query ranks first");
		// dimension mismatch yields nothing
		assert!(s.cold_search(&[1.0, 0.0, 0.0], 2).unwrap().is_empty());
	}

	#[test]
	fn cold_cap_drops_oldest() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		for (i, id) in ["old", "mid", "new"].iter().enumerate() {
			let mut e = mk_entity(id, id, 1.0, EntityKind::Claim);
			e.created_at = Some(UNIX_EPOCH + Duration::from_secs(100 * (i as u64 + 1)));
			s.cold_spill(&e).unwrap();
		}
		s.cold_cap(2).unwrap();
		assert!(s.cold_get("new").unwrap().is_some(), "newest kept");
		assert!(s.cold_get("mid").unwrap().is_some(), "second-newest kept");
		assert!(s.cold_get("old").unwrap().is_none(), "oldest evicted");
	}

	#[test]
	fn cold_search_breaks_cosine_ties_by_id_ascending() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		// Identical vectors; spill the higher id first so only the id tiebreak (not
		// scan/insert order) can pick the survivor of `truncate(1)`.
		let mut eb = mk_entity("b", "dup", 0.0, EntityKind::Claim);
		eb.vector = vec![1.0, 0.0];
		let mut ea = mk_entity("a", "dup", 0.0, EntityKind::Claim);
		ea.vector = vec![1.0, 0.0];
		s.cold_spill(&eb).unwrap();
		s.cold_spill(&ea).unwrap();

		let hits = s.cold_search(&[1.0, 0.0], 1).unwrap();
		assert_eq!(hits.len(), 1);
		assert_eq!(
			hits[0].0.id, "a",
			"equal-cosine tie resolved to id-ascending winner"
		);
	}
}
