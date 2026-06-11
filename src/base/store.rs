// SHARED-CANDIDATE — a content-addressed embedded store + zstd codec is exactly
// the durable primitive relay/agnt would also want. Kept kern-local for now by
// explicit decision (see docs/superpowers/plans/2026-06-10-redb-store.md); lift
// into `shared/` if a second daemon needs it rather than reinventing it.
//
// The durable substrate: one embedded LMDB environment per data_dir, replacing
// the legacy file-per-shard bincode tier (`persist.rs`) and the JSONL cold tier
// (`cold.rs`). Container swap only — the codec stays bincode, wrapped in zstd.
// Vectors are stored int8-on-disk (see `StoredVec`); `gnn_vector` is dropped on
// save and recomputed by `GnnPropagate` on demand.
//
// LMDB (via `heed`) is chosen over a single-process store (redb/sled) because
// kern's model has the per-cwd daemon AND the CLI AND the recall hook (`kern
// search`) all touch the same data dir concurrently. LMDB is multi-process by
// design: many concurrent readers + one writer, MVCC, mmap — readers never block
// the tick writer, and a second writer waits rather than failing. It is an
// in-process library (no network hop, no fallback backend), so it is a storage
// primitive like bincode, not a "pluggable backend" — it complies with the
// no-pluggable-backend repo law.

use std::collections::HashMap;
use std::path::Path;

use heed::types::{Bytes, Str};
use heed::{Database, Env, EnvOpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::base::types::{Entity, Kern};
use crate::quant::{QuantizationMode, QuantizedVec};

/// Max size of the LMDB memory map (and therefore the store's max on-disk size).
/// LMDB mmaps this virtual range up front; NTFS/most filesystems keep the backing
/// file sparse, so actual disk use tracks real data, not this cap. Bump if a
/// single project's graph + cold tier can exceed it.
const MAP_SIZE: usize = 4 * 1024 * 1024 * 1024; // 4 GiB
/// Named databases: kern, cold, meta.
const MAX_DBS: u32 = 3;

const KERN_DB: &str = "kern";
const COLD_DB: &str = "cold";
const META_DB: &str = "meta";
const META_KEY: &str = "graph";

/// Value-format version byte, prepended to every stored value ahead of the zstd
/// frame. A future on-disk format change bumps this so an old reader rejects a
/// new value loudly instead of mis-decoding it.
const FORMAT_V1: u8 = 1;
/// zstd compression level. 3 is the zstd default — a good ratio/speed balance
/// for the small, repetitive bincode blobs we store.
const ZSTD_LEVEL: i32 = 3;

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

/// Cap bincode-decoded allocations at 1 GiB — same guard as the legacy persist
/// layer: a corrupt/fuzzed length prefix can otherwise trick the decoder into
/// requesting petabytes. Real values are far smaller.
fn bincode_cfg() -> impl bincode::config::Config {
	bincode::config::standard().with_limit::<{ 1024 * 1024 * 1024 }>()
}

/// `[FORMAT_V1] ++ zstd(bincode(v))`.
fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>, StoreError> {
	let raw = bincode::serde::encode_to_vec(v, bincode_cfg())?;
	let comp = zstd::encode_all(raw.as_slice(), ZSTD_LEVEL)?;
	let mut out = Vec::with_capacity(comp.len() + 1);
	out.push(FORMAT_V1);
	out.extend_from_slice(&comp);
	Ok(out)
}

/// Inverse of [`encode`]. Rejects an unknown leading version byte rather than
/// feeding arbitrary bytes to zstd/bincode.
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
	let (&ver, body) = bytes.split_first().ok_or(StoreError::BadVersion(0))?;
	if ver != FORMAT_V1 {
		return Err(StoreError::BadVersion(ver));
	}
	let raw = zstd::decode_all(body)?;
	let (v, _) = bincode::serde::decode_from_slice(&raw, bincode_cfg())?;
	Ok(v)
}

/// A bincode-safe int8 vector: scale + quantized components, no skipped fields.
///
/// We deliberately do NOT persist [`QuantizedVec`] directly: it carries
/// `#[serde(skip_serializing_if = ...)]` on its `f`/`q` fields, which is a trap
/// under bincode — bincode is positional/non-self-describing, so a field omitted
/// on write desyncs the decoder (it still reads it) and corrupts everything
/// after. `StoredVec` has every field always-present, so encode and decode stay
/// in lockstep. Encoding reuses the tested int8 quantizer in `quant`.
#[derive(Serialize, Deserialize)]
pub struct StoredVec {
	pub scale: f32,
	pub q: Vec<i8>,
}

impl StoredVec {
	fn encode(v: &[f64]) -> Self {
		let qv = QuantizedVec::encode(v, QuantizationMode::Int8);
		StoredVec {
			scale: qv.scale,
			q: qv.q,
		}
	}

	fn decode(&self) -> Vec<f64> {
		self
			.q
			.iter()
			.map(|&x| (x as f64) * (self.scale as f64))
			.collect()
	}
}

/// On-disk projection of a [`Kern`]: the kern with every entity/reason vector
/// lifted out into int8 side-maps and `gnn_vector` dropped. Storing the heavy,
/// high-entropy float vectors as int8 (1 byte/dim vs 8) is the size win zstd
/// alone can't deliver on embeddings; `gnn_vector` is derived (GnnPropagate
/// recomputes it) so it is pure waste at rest.
#[derive(Serialize, Deserialize)]
pub struct StoredKern {
	pub kern: Kern,
	pub entity_vecs: HashMap<String, StoredVec>,
	pub reason_vecs: HashMap<String, StoredVec>,
}

impl StoredKern {
	pub fn from_kern(k: &Kern) -> Self {
		let mut kern = k.clone();
		let mut entity_vecs = HashMap::new();
		let mut reason_vecs = HashMap::new();
		for (id, e) in kern.entities.iter_mut() {
			if !e.vector.is_empty() {
				entity_vecs.insert(id.clone(), StoredVec::encode(&e.vector));
			}
			// Both float vectors are cleared so they don't bloat the bincode blob.
			// `vector` is restored from the int8 side-map on load; `gnn_vector` is
			// recomputed by GnnPropagate and intentionally never persisted.
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
		}
	}

	pub fn into_kern(self) -> Kern {
		let mut kern = self.kern;
		for (id, e) in kern.entities.iter_mut() {
			if let Some(q) = self.entity_vecs.get(id) {
				e.vector = q.decode();
			}
			// gnn_vector stays empty — recomputed lazily.
		}
		for (id, r) in kern.reasons.iter_mut() {
			if let Some(q) = self.reason_vecs.get(id) {
				r.vector = q.decode();
			}
		}
		kern
	}
}

#[derive(Serialize, Deserialize)]
struct GraphMeta {
	network_id: String,
	quant_mode: QuantizationMode,
}

/// One embedded LMDB environment per `data_dir`. The `Env` handle is internally
/// reference-counted and cheap to clone; database handles are `Copy`. LMDB gives
/// many-reader / single-writer concurrency across processes, so concurrent
/// recalls (CLI, hook, daemon) read a consistent snapshot without blocking the
/// tick writer.
pub struct Store {
	env: Env,
	kern: Database<Str, Bytes>,
	cold: Database<Str, Bytes>,
	meta: Database<Str, Bytes>,
}

impl Store {
	/// Open (creating if absent) the LMDB environment under `dir`. LMDB writes a
	/// `data.mdb` + `lock.mdb` into the directory. All named databases are created
	/// up front so later read transactions never miss a database on a fresh env.
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
		})
	}

	// ---- generic typed KV (used by the graph-level helpers + tests) ----

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
		let rtxn = self.env.read_txn()?;
		match db.get(&rtxn, key)? {
			Some(b) => Ok(Some(decode(b)?)),
			None => Ok(None),
		}
	}

	fn remove(&self, db: Database<Str, Bytes>, key: &str) -> Result<(), StoreError> {
		let mut wtxn = self.env.write_txn()?;
		db.delete(&mut wtxn, key)?;
		wtxn.commit()?;
		Ok(())
	}

	/// Decode every row of `db`. Rows that fail to decode are skipped with a
	/// warning rather than failing the whole scan — a single corrupt value must
	/// not blind the daemon to the rest of the graph.
	fn scan<T: DeserializeOwned>(
		&self,
		db: Database<Str, Bytes>,
	) -> Result<Vec<(String, T)>, StoreError> {
		let rtxn = self.env.read_txn()?;
		let mut out = Vec::new();
		for item in db.iter(&rtxn)? {
			let (k, v) = item?;
			match decode::<T>(v) {
				Ok(val) => out.push((k.to_string(), val)),
				Err(e) => {
					tracing::warn!(target: "kern.store", key = %k, error = %e, "skipping corrupt value");
				}
			}
		}
		Ok(out)
	}

	// ---- graph-level save / load ----

	/// Persist the whole graph in one write transaction: every live kern, prune
	/// any kern row no longer in the live set (replaces `save_all`'s orphan
	/// reconcile), and the graph metadata row. One atomic commit — a crash leaves
	/// either the old or the new graph, never a torn mix of shards.
	pub fn save_all_kerns(
		&self,
		kerns: &HashMap<String, Kern>,
		network_id: &str,
		quant_mode: QuantizationMode,
	) -> Result<(), StoreError> {
		let mut wtxn = self.env.write_txn()?;
		// Collect existing keys first (immutable borrow of the txn), then mutate —
		// can't hold the iterator borrow across put/delete.
		let existing: Vec<String> = {
			let mut v = Vec::new();
			for item in self.kern.iter(&wtxn)? {
				let (k, _) = item?;
				v.push(k.to_string());
			}
			v
		};
		for id in existing {
			if !kerns.contains_key(&id) {
				self.kern.delete(&mut wtxn, id.as_str())?;
			}
		}
		for (id, kern) in kerns {
			let bytes = encode(&StoredKern::from_kern(kern))?;
			self.kern.put(&mut wtxn, id.as_str(), &bytes)?;
		}
		let meta = GraphMeta {
			network_id: network_id.to_string(),
			quant_mode,
		};
		let meta_bytes = encode(&meta)?;
		self.meta.put(&mut wtxn, META_KEY, &meta_bytes)?;
		wtxn.commit()?;
		Ok(())
	}

	/// Load every kern plus the graph metadata. Corrupt kern rows are skipped
	/// with a warning (the rest of the graph still loads). Missing metadata
	/// yields an empty network_id + `QuantizationMode::None`, which the caller
	/// backfills.
	pub fn load_all_kerns(
		&self,
	) -> Result<(HashMap<String, Kern>, String, QuantizationMode), StoreError> {
		let stored: Vec<(String, StoredKern)> = self.scan(self.kern)?;
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

	/// Persist a single kern (the tick worker's per-kern `do_persist` path).
	pub fn save_one_kern(&self, kern: &Kern) -> Result<(), StoreError> {
		self.put(self.kern, &kern.id.clone(), &StoredKern::from_kern(kern))
	}

	/// Load a single kern by id (the lazy-load path for an unloaded kern).
	pub fn load_one_kern(&self, id: &str) -> Result<Option<Kern>, StoreError> {
		Ok(
			self
				.get::<StoredKern>(self.kern, id)?
				.map(StoredKern::into_kern),
		)
	}

	/// Delete a single kern row (deregister). Idempotent — a missing row is fine.
	pub fn delete_one_kern(&self, id: &str) -> Result<(), StoreError> {
		self.remove(self.kern, id)
	}

	// ---- cold tier ----

	/// Spill an evicted entity to the cold database, then enforce the size cap. A
	/// put overwrites any prior row for the same id (latest-wins), so the cold
	/// tier never accumulates duplicate rows the way the JSONL append log did.
	pub fn cold_spill(&self, entity: &Entity) -> Result<(), StoreError> {
		self.put(self.cold, &entity.id.clone(), entity)?;
		self.cold_cap(crate::base::constants::COLD_MAX_ENTRIES)?;
		Ok(())
	}

	/// Fetch one cold entity by id.
	pub fn cold_get(&self, id: &str) -> Result<Option<Entity>, StoreError> {
		self.get(self.cold, id)
	}

	/// Every cold entity (used by `reembed` to re-vector the whole cold tier).
	pub fn cold_all(&self) -> Result<Vec<Entity>, StoreError> {
		Ok(self.scan(self.cold)?.into_iter().map(|(_, e)| e).collect())
	}

	/// Insert/replace many cold entities in one transaction, then cap once. Used
	/// by `reembed`'s write-back: a per-entity `cold_spill` would fsync a separate
	/// commit per row (thousands of them), where this commits the whole batch once.
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

	/// Top-`k` cold entities by cosine similarity to `query_vec`, descending.
	/// Rows whose stored vector is empty or a different dimension are skipped.
	/// The cold tier is bounded by [`COLD_MAX_ENTRIES`](crate::base::constants::COLD_MAX_ENTRIES),
	/// so the full decode-and-score scan is bounded work.
	pub fn cold_search(&self, query_vec: &[f64], k: usize) -> Result<Vec<(Entity, f64)>, StoreError> {
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
		scored.sort_by(|a, b| crate::base::util::cmp_partial(&b.1, &a.1));
		scored.truncate(k);
		Ok(scored)
	}

	/// Cap the cold tier at `max` rows, dropping the oldest by `created_at` (rows
	/// with no timestamp sort oldest and go first). No-op while under cap, so the
	/// common spill path pays only a cheap `len()` check.
	fn cold_cap(&self, max: usize) -> Result<(), StoreError> {
		let len = {
			let rtxn = self.env.read_txn()?;
			self.cold.len(&rtxn)? as usize
		};
		if len <= max {
			return Ok(());
		}
		// Over cap: decode all to read created_at, keep the newest `max`.
		let mut rows: Vec<(String, Entity)> = self.scan(self.cold)?;
		rows.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
		let drop_ids: Vec<String> = rows.into_iter().skip(max).map(|(id, _)| id).collect();
		let mut wtxn = self.env.write_txn()?;
		for id in &drop_ids {
			self.cold.delete(&mut wtxn, id.as_str())?;
		}
		wtxn.commit()?;
		Ok(())
	}
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

	// ---- codec ----

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
		assert_eq!(bytes[0], FORMAT_V1, "first byte is the format version");
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

	// ---- generic KV ----

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

	// ---- StoredKern projection ----

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
			assert!((got - want).abs() < 0.02, "int8 within tolerance: {got} vs {want}");
		}
		assert!(be.gnn_vector.is_empty(), "gnn_vector is dropped, not persisted");
		assert_eq!(be.heat, 1.0, "non-vector fields survive");
		assert_eq!(be.text(), "a fact");
	}

	#[test]
	fn stored_kern_handles_empty_vectors() {
		let e = mk_entity("e1", "novec", 0.0, EntityKind::Claim);
		// mk_entity gives a zero vector; clear it to exercise the empty path.
		let mut e = e;
		e.vector = Vec::new();
		let k = kern_with("k", e);
		let sk = StoredKern::from_kern(&k);
		assert!(sk.entity_vecs.is_empty(), "no side-map entry for an empty vector");
		let back = sk.into_kern();
		assert!(!back.entities["e1"].has_vector());
	}

	// ---- graph-level save / load ----

	#[test]
	fn save_then_load_graph_roundtrip() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let mut e = mk_entity("e1", "hello", 2.0, EntityKind::Fact);
		e.vector = vec![0.5, -0.5, 0.25];
		let mut kerns = HashMap::new();
		kerns.insert("root".to_string(), Kern::new("root", ""));
		kerns.insert("k".to_string(), kern_with("k", e));

		s.save_all_kerns(&kerns, "net-123", QuantizationMode::Int8).unwrap();
		let (loaded, net, qm) = s.load_all_kerns().unwrap();

		assert_eq!(net, "net-123");
		assert_eq!(qm, QuantizationMode::Int8);
		assert_eq!(loaded.len(), 2);
		let be = &loaded["k"].entities["e1"];
		assert_eq!(be.text(), "hello");
		assert!((be.vector[0] - 0.5).abs() < 0.02);
	}

	#[test]
	fn save_all_prunes_removed_kerns() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		let mut kerns = HashMap::new();
		kerns.insert("a".to_string(), Kern::new("a", ""));
		kerns.insert("b".to_string(), Kern::new("b", ""));
		s.save_all_kerns(&kerns, "n", QuantizationMode::None).unwrap();

		kerns.remove("b");
		s.save_all_kerns(&kerns, "n", QuantizationMode::None).unwrap();

		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(loaded.contains_key("a"));
		assert!(!loaded.contains_key("b"), "removed kern pruned from disk");
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
		// idempotent
		s.delete_one_kern("k").unwrap();
	}

	#[test]
	fn corrupt_kern_value_is_skipped() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		s.save_one_kern(&kern_with("good", mk_entity("e", "ok", 0.0, EntityKind::Claim)))
			.unwrap();
		// Inject a corrupt raw value under a sibling key.
		{
			let mut wtxn = s.env.write_txn().unwrap();
			s.kern.put(&mut wtxn, "bad", b"not a valid value".as_slice()).unwrap();
			wtxn.commit().unwrap();
		}
		let (loaded, _, _) = s.load_all_kerns().unwrap();
		assert!(loaded.contains_key("good"), "valid kern loads");
		assert!(!loaded.contains_key("bad"), "corrupt kern skipped, not fatal");
	}

	// ---- cold tier ----

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
		s.cold_spill(&mk_entity("x", "v1", 1.0, EntityKind::Claim)).unwrap();
		s.cold_spill(&mk_entity("x", "v2", 5.0, EntityKind::Claim)).unwrap();
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
		// Force the cap below the row count.
		s.cold_cap(2).unwrap();
		assert!(s.cold_get("new").unwrap().is_some(), "newest kept");
		assert!(s.cold_get("mid").unwrap().is_some(), "second-newest kept");
		assert!(s.cold_get("old").unwrap().is_none(), "oldest evicted");
	}
}
