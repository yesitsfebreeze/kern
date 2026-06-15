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
use heed::{CompactionOption, Database, Env, EnvOpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::base::types::{Entity, Kern};
use crate::quant::{QuantizationMode, QuantizedVec};

/// Max size of the LMDB memory map (and therefore the store's max on-disk size).
/// LMDB mmaps this virtual range up front; NTFS/most filesystems keep the backing
/// file sparse, so actual disk use tracks real data, not this cap. On 64-bit the
/// reservation is virtual address space (near-free), so this is set generously.
///
/// Headroom is a DURABILITY requirement, not just a growth allowance: when the env
/// fills, LMDB returns `MDB_MAP_FULL` even for the *deletes* that would free space
/// (a copy-on-write B-tree needs free pages to commit a deletion). The startup
/// liveness reap (`gc_empty_kerns`) is exactly such a bulk delete — under the old
/// 4 GiB cap, an env bloated to the limit by the unnamed-child spawn runaway could
/// not commit its own cleanup (observed: 178k empty kerns reaped in memory but the
/// save failed `MDB_MAP_FULL`, so the bloat never cleared and every restart
/// re-failed). The cap must stay well above the largest transient bloat so the
/// graph can always self-heal. Bump further if a real graph + cold tier approaches
/// it. (Kept moderate rather than enormous: on Windows not every filesystem keeps
/// the LMDB map sparse, so the cap also bounds worst-case file size.)
const MAP_SIZE: usize = 16 * 1024 * 1024 * 1024; // 16 GiB
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
	/// The data directory this env lives in. Held so the offline compactor can
	/// locate `data.mdb`/`lock.mdb` for the copy-and-swap.
	dir: std::path::PathBuf,
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
			dir: path.to_path_buf(),
		})
	}

	/// Apparent size of the backing `data.mdb` in bytes (the LMDB high-water mark).
	/// LMDB never returns freed pages to the OS, so after a bulk delete this stays
	/// at its peak until [`compact_dir`] rewrites the env. Used to decide whether a
	/// compaction is worth its cost. Returns 0 if the file can't be stat'd.
	pub fn data_file_len(&self) -> u64 {
		std::fs::metadata(self.dir.join("data.mdb"))
			.map(|m| m.len())
			.unwrap_or(0)
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
		// Cosine descending, ties broken by entity id ascending. The id tiebreak
		// makes the truncation deterministic — the cold rows come from an LMDB scan
		// whose order must not decide which equal-cosine entities survive `take k`.
		scored.sort_by(|a, b| crate::base::util::cmp_rank(a.1, &a.0.id, b.1, &b.0.id));
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

/// The sidecar path a compacted copy is written to before the swap.
fn compact_tmp(dir: &Path) -> std::path::PathBuf {
	dir.join("data.mdb.compact")
}

/// Replace `dir/data.mdb` with the compacted copy at `dir/data.mdb.compact` and
/// drop the stale `lock.mdb`. Returns `(old_bytes, new_bytes)`.
///
/// The caller MUST have dropped every [`Store`]/`Env` handle to `dir` first. Even
/// then, heed/LMDB unmaps the file ASYNCHRONOUSLY on Windows, so a rename issued
/// immediately can still hit `Access is denied` while the unmap drains. We retry
/// with a short backoff to ride out that lag rather than failing the whole compaction.
pub fn swap_compacted(dir: &str) -> Result<(u64, u64), StoreError> {
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
				// Backoff up to ~2.5s total while the OS releases the old mmap.
				std::thread::sleep(std::time::Duration::from_millis(100 + attempt * 4));
			}
		}
	}
	// Give up: clean the tmp so a retry isn't confused by a stale copy.
	let _ = std::fs::remove_file(&tmp);
	Err(StoreError::Io(last_err.unwrap_or_else(|| {
		std::io::Error::other("compaction swap failed")
	})))
}

/// Compact the LMDB env under `dir` in place, reclaiming the disk that a bulk
/// delete (e.g. the empty-kern reap) freed inside the file but never returned to
/// the OS. LMDB only ever grows `data.mdb` to a high-water mark; deleted pages
/// are reused for future writes but the file stays at peak size (and on Windows
/// NTFS it is not even sparse, so that peak is real disk). The only way to shrink
/// it is to rewrite the live data into a fresh env.
///
/// Opens its OWN env, writes the compacted copy, closes the env deterministically
/// (`prepare_for_closing().wait()`), then swaps. REQUIRES exclusive access — no
/// daemon or other process may have the env open. Run from an offline command
/// with the daemon stopped. Returns `(old_bytes, new_bytes)`.
pub fn compact_dir(dir: &str) -> Result<(u64, u64), StoreError> {
	let path = Path::new(dir);
	let tmp = compact_tmp(path);
	let _ = std::fs::remove_file(&tmp); // clear any stale tmp

	{
		let env = unsafe {
			EnvOpenOptions::new()
				.map_size(MAP_SIZE)
				.max_dbs(MAX_DBS)
				.open(path)?
		};
		env.copy_to_file(&tmp, CompactionOption::Enabled)?;
		// Block until the env is truly closed (mmap released, handles shut).
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
		// mk_entity gives a zero vector; clear it to exercise the empty path.
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
	fn compact_dir_shrinks_after_bulk_delete() {
		let d = tmp();
		let dir = dir_of(&d);
		// Grow the env well past its initial pages: write many fat kern rows so the
		// file's high-water mark climbs, then delete almost all of them.
		{
			let s = Store::open(&dir).unwrap();
			let mut kerns = HashMap::new();
			kerns.insert("root".to_string(), Kern::new("root", ""));
			for i in 0..2000 {
				let mut e = mk_entity(&format!("e{i}"), &"x".repeat(512), 1.0, EntityKind::Claim);
				e.vector = (0..256).map(|j| ((i + j) as f64).sin()).collect();
				kerns.insert(format!("k{i}"), kern_with(&format!("k{i}"), e));
			}
			s.save_all_kerns(&kerns, "n", QuantizationMode::Int8)
				.unwrap();
			let bloated = s.data_file_len();

			// Reap down to just root + one survivor and persist the deletion.
			let mut small = HashMap::new();
			small.insert("root".to_string(), Kern::new("root", ""));
			small.insert("k0".to_string(), kerns.remove("k0").unwrap());
			s.save_all_kerns(&small, "n", QuantizationMode::Int8)
				.unwrap();
			// The delete frees pages INSIDE the file but does not return them to the
			// OS, so the file stays at (essentially) its high-water mark — that is the
			// whole bug. It must remain far larger than the handful of live rows need.
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

		// Live data survives the rewrite.
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
		// idempotent
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
		// Inject a corrupt raw value under a sibling key.
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
		// Force the cap below the row count.
		s.cold_cap(2).unwrap();
		assert!(s.cold_get("new").unwrap().is_some(), "newest kept");
		assert!(s.cold_get("mid").unwrap().is_some(), "second-newest kept");
		assert!(s.cold_get("old").unwrap().is_none(), "oldest evicted");
	}

	#[test]
	fn cold_search_breaks_cosine_ties_by_id_ascending() {
		let d = tmp();
		let s = Store::open(&dir_of(&d)).unwrap();
		// Identical vectors -> identical cosine to the query. Spill the higher id
		// first so only the id tiebreak (not scan/insert order) can pick the winner
		// that survives `truncate(1)`. This pins the deterministic-ranking contract.
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
