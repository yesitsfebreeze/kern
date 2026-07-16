use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use super::constants::KERN_CAP_DISABLED;
use super::lexical::LexicalIndex;
use super::store::{Store, StoreError};
use super::types::{EntityStatus, Kern};
use super::util;
use super::vector_backend::VectorBackend;
use crate::quant::QuantizationMode;

/// Insert every entity/reason/source of `kern` into the cross-kern lookup
/// maps and vector indices. Taken as disjoint `&mut` fields (not `&mut self`)
/// so the caller can iterate `self.kerns` while filling the indices. Single
/// source for the index-population loop shared by `rebuild_index` and the
/// lazy-load path in `get`.
#[allow(clippy::too_many_arguments)]
fn index_kern_into(
	kern: &Kern,
	entity_kern: &mut HashMap<String, String>,
	reason_kern: &mut HashMap<String, String>,
	src_index: &mut HashMap<String, String>,
	// `None` skips entity-vector insertion — used by `rebuild_index` when the
	// entity index is a disk `snapshot` that ALREADY holds every resident entity
	// (re-inserting would tombstone the whole snapshot into the delta). The reverse
	// `entity_kern` map is still populated regardless. The lazy-load path passes
	// `Some`, so a newly loaded kern's entities enter the live index (the delta,
	// when disk-backed — correct, since they were not in the snapshot).
	mut entity_idx: Option<&mut VectorBackend>,
	gnn_entity_idx: &mut VectorBackend,
	reason_idx: &mut VectorBackend,
) {
	for t in kern.entities.values() {
		entity_kern.insert(t.id.clone(), kern.id.clone());
		// A Superseded entity can never be a retrieval result (`retrieval::score`
		// drops it), so it must not enter the ANN search indices on load/rebuild —
		// otherwise it burns top-k candidate slots and index memory only to be
		// filtered downstream (the durable half of the supersede index-removal; the
		// live transition is handled in `accept::supersede`). Its `entity_kern`
		// mapping is kept so the supersede chain still resolves.
		let searchable = t.status != EntityStatus::Superseded;
		if searchable && t.has_vector() {
			if let Some(ei) = entity_idx.as_deref_mut() {
				ei.insert(t.id.clone(), t.vector.clone());
			}
		}
		if searchable && t.has_gnn_vector() {
			gnn_entity_idx.insert(t.id.clone(), t.gnn_vector.clone());
		}
	}
	for r in kern.reasons.values() {
		reason_kern.insert(r.id.clone(), kern.id.clone());
		if r.has_vector() {
			reason_idx.insert(r.id.clone(), r.vector.clone());
		}
	}
	for ext_id in kern.source_index.keys() {
		src_index.insert(ext_id.clone(), kern.id.clone());
	}
}

pub struct GraphGnn {
	pub root: Kern,
	pub network_id: String,
	pub data_dir: String,
	/// The embedded LMDB store backing this graph. `None` for an in-memory graph
	/// (empty `data_dir`, e.g. tests). Opened once per load and shared so the
	/// process holds a single LMDB env handle (LMDB forbids opening one env twice
	/// in a process). Cheap to clone — it is reference-counted.
	store: Option<Arc<Store>>,
	pub quant_mode: QuantizationMode,
	pub gnn_entity_idx: VectorBackend,
	pub entity_idx: VectorBackend,
	pub reason_idx: VectorBackend,
	pub kerns: HashMap<String, Kern>,
	unloaded: HashSet<String>,
	src_index: HashMap<String, String>,
	entity_kern: HashMap<String, String>,
	reason_kern: HashMap<String, String>,
	lexical: Option<Arc<LexicalIndex>>,
	/// Soft cap on the number of kerns held in memory at once. When `register`
	/// would exceed this, the LRU (oldest `last_access`) non-root kern is
	/// `unload`ed to disk. [`KERN_CAP_DISABLED`] disables the cap.
	max_loaded_kerns: usize,
	/// Resident searchable-entity count above which `rebuild_index` spills the
	/// entity index to a disk-resident DiskANN snapshot (see
	/// [`GraphConfig::disk_threshold`](crate::config::graph::GraphConfig)).
	/// [`KERN_CAP_DISABLED`] (the default) means never spill.
	disk_threshold: usize,
	/// Monotonic graph-wide mutation counter, bumped on every content mutation:
	/// a kern handed out mutably (`get_mut`), registered, or deregistered. The
	/// query result cache stamps each entry with the epoch at creation and treats
	/// the entry as valid only while the epoch is unchanged.
	///
	/// A *global* epoch rather than per-kern versions is deliberate and required
	/// for soundness: HyDE rewrites the query before search, so retrieval reaches
	/// kerns far from the raw query vector. A new memory landing in a kern the
	/// previous run never touched would be invisible to any per-kern dependency
	/// set, so that query would keep serving a result omitting it. Invalidating on
	/// *any* mutation removes that hole — between writes the cache hits fully; a
	/// write conservatively flushes it. Runtime-only (never serialised); the read/
	/// query path takes `&GraphGnn` and so can never bump it — only mutations do.
	mutation_epoch: u64,
	/// Store write-generation this graph is known consistent with — the on-disk
	/// [`epoch`](crate::base::store) value at the last load or successful flush. The
	/// persist paths compare it against the live on-disk epoch to detect that
	/// ANOTHER writer (a stray CLI direct-write, a second daemon) committed since,
	/// so a stale snapshot is never written over newer committed rows. The in-RAM
	/// graph (no store) keeps this at 0. Runtime-only; never serialised.
	flushed_epoch: u64,
	/// Dense entity index + out-edge adjacency, built lazily by
	/// [`entity_adjacency`](Self::entity_adjacency) and reused while the
	/// `mutation_epoch` stamp matches. Runtime-only; never serialised.
	adjacency_cache: parking_lot::RwLock<Option<(u64, Arc<EntityAdjacency>)>>,
}

/// Snapshot of the entity graph as dense indices: every distinct entity id gets
/// an index into `ids`, and `out[i]` lists the indices entity `i` points at via
/// its reasons (self-loops and edges touching unknown entities skipped).
pub struct EntityAdjacency {
	pub id_to_idx: HashMap<String, usize>,
	pub ids: Vec<String>,
	pub out: Vec<Vec<usize>>,
}

impl EntityAdjacency {
	fn build(g: &GraphGnn) -> Self {
		let mut id_to_idx: HashMap<String, usize> = HashMap::new();
		let mut ids: Vec<String> = Vec::new();
		for kern in g.map().values() {
			for t in kern.entities.values() {
				if !id_to_idx.contains_key(&t.id) {
					id_to_idx.insert(t.id.clone(), ids.len());
					ids.push(t.id.clone());
				}
			}
		}
		let mut out: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
		for kern in g.map().values() {
			for r in kern.reasons.values() {
				if r.from == r.to {
					continue;
				}
				let (Some(&fi), Some(&ti)) = (id_to_idx.get(&r.from), id_to_idx.get(&r.to)) else {
					continue;
				};
				out[fi].push(ti);
			}
		}
		Self { id_to_idx, ids, out }
	}
}

impl Default for GraphGnn {
	fn default() -> Self {
		Self::new()
	}
}

impl GraphGnn {
	pub fn new() -> Self {
		let mut root = Kern::new_root();
		let network_id = util::uuid_v4();
		root.root_id = network_id.clone();
		let root_id = root.id.clone();
		let mut kerns = HashMap::new();
		kerns.insert(root_id, root.clone());
		let quant_mode = QuantizationMode::default();
		Self {
			root,
			network_id,
			data_dir: String::new(),
			store: None,
			quant_mode,
			entity_idx: VectorBackend::resident(16, 200, quant_mode),
			gnn_entity_idx: VectorBackend::resident(16, 200, quant_mode),
			reason_idx: VectorBackend::resident(16, 200, quant_mode),
			kerns,
			unloaded: HashSet::new(),
			src_index: HashMap::new(),
			entity_kern: HashMap::new(),
			reason_kern: HashMap::new(),
			lexical: Some(Arc::new(LexicalIndex::new_in_ram(1.2, 0.75))),
			max_loaded_kerns: KERN_CAP_DISABLED,
			disk_threshold: KERN_CAP_DISABLED,
			mutation_epoch: 0,
			flushed_epoch: 0,
			adjacency_cache: parking_lot::RwLock::new(None),
		}
	}

	/// The store write-generation this graph last loaded or flushed at. Persist
	/// paths pass it to the stale-flush guard so they never overwrite rows a
	/// concurrent writer committed after this graph's snapshot was taken.
	pub fn flushed_epoch(&self) -> u64 {
		self.flushed_epoch
	}

	/// Record the store epoch this graph is now consistent with (after a load or a
	/// successful guarded flush). Not a content mutation, so it does NOT bump the
	/// query-cache `mutation_epoch`.
	pub fn set_flushed_epoch(&mut self, epoch: u64) {
		self.flushed_epoch = epoch;
	}

	pub fn set_max_loaded_kerns(&mut self, cap: usize) {
		self.max_loaded_kerns = cap.max(1);
	}

	/// Set the resident-entity count above which the entity index spills to a disk
	/// DiskANN snapshot on the next [`rebuild_index`](Self::rebuild_index).
	/// [`KERN_CAP_DISABLED`] disables spilling. Takes effect on the next rebuild.
	pub fn set_disk_threshold(&mut self, threshold: usize) {
		self.disk_threshold = threshold;
	}

	/// Bind this graph to an open LMDB store. Called once after load so the
	/// lazy-load / unload / deregister / persist paths share a single env handle.
	pub fn set_store(&mut self, store: Arc<Store>) {
		self.store = Some(store);
	}

	/// The store handle, if this graph is disk-backed. Cloned (ref-counted) so
	/// callers can use it without holding a borrow on the graph.
	pub fn store(&self) -> Option<Arc<Store>> {
		self.store.clone()
	}

	/// Evict the oldest non-root kern by `last_access` while we are over the
	/// soft cap. Errors during `unload` (persist failures) are swallowed —
	/// the caller already accepted that we may degrade under pressure.
	fn enforce_kern_cap(&mut self) {
		if self.max_loaded_kerns == KERN_CAP_DISABLED {
			return;
		}
		while self.kerns.len() > self.max_loaded_kerns {
			let root_id = self.root.id.clone();
			let victim = self
				.kerns
				.iter()
				.filter(|(id, _)| **id != root_id)
				.min_by_key(|(_, k)| k.last_access.unwrap_or(SystemTime::UNIX_EPOCH))
				.map(|(id, _)| id.clone());
			match victim {
				Some(id) => {
					let _ = self.unload(&id);
				}
				None => break,
			}
		}
	}

	pub fn lexical(&self) -> Option<Arc<LexicalIndex>> {
		self.lexical.clone()
	}

	pub fn set_lexical(&mut self, lex: Option<Arc<LexicalIndex>>) {
		self.lexical = lex;
	}

	pub fn rebuild_index(&mut self) {
		self.gnn_entity_idx = VectorBackend::resident(16, 200, self.quant_mode);
		self.reason_idx = VectorBackend::resident(16, 200, self.quant_mode);
		self.src_index.clear();
		self.entity_kern.clear();
		self.reason_kern.clear();

		// Choose the entity backend. Above `disk_threshold` (and with a data_dir to
		// write to) the entity vectors spill to a disk-resident DiskANN snapshot,
		// keeping them off the heap; otherwise the historical in-RAM HNSW. The
		// default threshold is the disabled sentinel, so this is a no-op for small
		// deployments. A build/open failure falls back to the in-RAM index — the
		// graph stays searchable, never broken, on a disk error.
		let entity_count = self.resident_searchable_entity_count();
		let spill = !self.data_dir.is_empty() && entity_count > self.disk_threshold;
		self.entity_idx = match spill.then(|| self.build_entity_disk_snapshot()).flatten() {
			Some(snapshot) => VectorBackend::disk(snapshot, self.quant_mode),
			None => VectorBackend::resident(16, 200, self.quant_mode),
		};

		// When the entity index is a disk snapshot it ALREADY holds every resident
		// entity, so the populate loop must not re-insert them (that would tombstone
		// the whole snapshot into the delta). `None` skips entity insertion while
		// still filling the reverse maps and the gnn/reason indices.
		let skip_entity_insert = matches!(self.entity_idx, VectorBackend::Disk { .. });
		for kern in self.kerns.values() {
			index_kern_into(
				kern,
				&mut self.entity_kern,
				&mut self.reason_kern,
				&mut self.src_index,
				(!skip_entity_insert).then_some(&mut self.entity_idx),
				&mut self.gnn_entity_idx,
				&mut self.reason_idx,
			);
		}
	}

	/// Count of resident entities eligible for the entity index — non-Superseded
	/// and vector-bearing, mirroring [`index_kern_into`]'s filter. Cheap (no
	/// vector clones); drives the [`rebuild_index`](Self::rebuild_index) spill
	/// decision.
	fn resident_searchable_entity_count(&self) -> usize {
		self
			.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| t.status != EntityStatus::Superseded && t.has_vector())
			.count()
	}

	/// Collect every searchable resident entity's content vector as `(id, f32)`,
	/// deduped by id and id-sorted (via `BTreeMap`) so the seeded Vamana build is
	/// reproducible. Mirrors `entity_idx` membership exactly; entity vectors are
	/// f32 end-to-end, so this is a plain clone.
	fn collect_entity_items(&self) -> Vec<(String, Vec<f32>)> {
		let mut items: std::collections::BTreeMap<String, Vec<f32>> = std::collections::BTreeMap::new();
		for kern in self.kerns.values() {
			for t in kern.entities.values() {
				if t.status != EntityStatus::Superseded && t.has_vector() {
					items.insert(t.id.clone(), t.vector.clone());
				}
			}
		}
		items.into_iter().collect()
	}

	/// Snapshot every searchable resident entity into a disk-resident Vamana index
	/// under `dir` (see [`collect_entity_items`](Self::collect_entity_items)).
	/// Returns the number of vectors written. The build half of the DiskANN
	/// integration; used by [`rebuild_index`](Self::rebuild_index) when spilling.
	pub fn build_entity_disk_index(&self, dir: &std::path::Path) -> std::io::Result<usize> {
		super::diskann::build_and_save(
			dir,
			&self.collect_entity_items(),
			super::diskann::Params::default(),
		)
	}

	/// Build the entity snapshot under `<data_dir>/diskann/entity` and open it.
	/// Returns `None` (logging a warning) on any build/open failure so the caller
	/// can fall back to the in-RAM index rather than break the graph.
	fn build_entity_disk_snapshot(&self) -> Option<super::diskann::DiskIndex> {
		let dir = std::path::Path::new(&self.data_dir)
			.join("diskann")
			.join("entity");
		if let Err(e) = self.build_entity_disk_index(&dir) {
			tracing::warn!(target: "kern.diskann", error = %e, "entity snapshot build failed; using in-RAM index");
			return None;
		}
		match super::diskann::DiskIndex::open(&dir) {
			Ok(idx) => Some(idx),
			Err(e) => {
				tracing::warn!(target: "kern.diskann", error = %e, "entity snapshot open failed; using in-RAM index");
				None
			}
		}
	}

	/// Rebuild the entity disk snapshot from the current resident entities and
	/// reset the in-RAM delta and tombstones. No-op unless the entity index is
	/// disk-backed. Without this the delta grows without bound on a long-running
	/// daemon (every post-snapshot write buffers there); folding it back keeps the
	/// resident overlay small. The source of truth is `self.kerns` — every delta
	/// entity is also stored there — so this re-snapshots from kerns (identical
	/// live membership, Superseded/forgotten ids naturally dropped), not from the
	/// delta. A build failure falls back to a correct in-RAM rebuild.
	///
	/// COST: the full Vamana `build_and_save` runs inline, and the tick dispatcher
	/// (`do_disk_consolidate`) holds the graph WRITE lock for its whole duration —
	/// a maintenance pause that scales with the resident entity count, blocking all
	/// reads/writes meanwhile. This matches the tick's serialized model (every task
	/// holds the write lock), and it is gated to fire at most hourly and only past
	/// `DISK_CONSOLIDATE_MIN_DELTA`, so it is rare; but on a very large corpus it is
	/// a stop-the-world stall. A non-blocking two-phase consolidate (build outside
	/// the lock against a transitional write-routing delta, swap under a brief lock)
	/// is the planned follow-up — see the backlog in
	/// `docs/superpowers/plans/2026-06-12-diskann-wiring.md`.
	pub fn consolidate_disk_index(&mut self) {
		if !matches!(self.entity_idx, VectorBackend::Disk { .. }) {
			return;
		}
		// Drop the old snapshot's mmap FIRST so the rebuild can overwrite its files
		// (Windows locks memory-mapped files). A transient empty resident index
		// stands in; the caller holds the graph write lock, so no search sees it.
		self.entity_idx = VectorBackend::resident(16, 200, self.quant_mode);
		match self.build_entity_disk_snapshot() {
			Some(snapshot) => self.entity_idx = VectorBackend::disk(snapshot, self.quant_mode),
			// Build failed mid-consolidate: repopulate a correct in-RAM index so the
			// graph is never left with the empty placeholder above.
			None => self.rebuild_index(),
		}
	}

	/// Post-snapshot writes currently buffered in the disk delta (0 if the entity
	/// index is not disk-backed). Drives the consolidation cadence.
	pub fn pending_disk_delta_len(&self) -> usize {
		self.entity_idx.pending_delta_len()
	}

	pub fn get(&mut self, id: &str) -> Option<&Kern> {
		if self.kerns.contains_key(id) {
			if let Some(k) = self.kerns.get_mut(id) {
				k.last_access = Some(SystemTime::now());
			}
			return self.kerns.get(id);
		}
		if self.unloaded.contains(id) {
			let loaded = self
				.store
				.clone()
				.and_then(|s| s.load_one_kern(id).ok().flatten());
			if let Some(mut k) = loaded {
				migrate_root_id(&mut k, &self.network_id);
				k.last_access = Some(SystemTime::now());
				index_kern_into(
					&k,
					&mut self.entity_kern,
					&mut self.reason_kern,
					&mut self.src_index,
					Some(&mut self.entity_idx),
					&mut self.gnn_entity_idx,
					&mut self.reason_idx,
				);
				self.unloaded.remove(id);
				self.kerns.insert(id.to_string(), k);
				return self.kerns.get(id);
			}
		}
		None
	}

	pub fn get_mut(&mut self, id: &str) -> Option<&mut Kern> {
		if !self.kerns.contains_key(id) {
			self.get(id);
		}
		if self.kerns.contains_key(id) {
			// Conservatively bump: a caller asking for `&mut Kern` is presumed to
			// mutate it. Over-bumping (a get_mut that changes nothing) only costs a
			// cache flush; it never serves stale data. Heat/access updates run on
			// result copies, not through here, so the read path never bumps.
			self.bump_mutation_epoch();
		}
		if let Some(k) = self.kerns.get_mut(id) {
			k.last_access = Some(SystemTime::now());
			Some(k)
		} else {
			None
		}
	}

	/// Advance the graph-wide mutation epoch. The query cache compares the stamped
	/// epoch against the live one to invalidate every entry on any change.
	pub fn bump_mutation_epoch(&mut self) {
		self.mutation_epoch = self.mutation_epoch.wrapping_add(1);
	}

	/// Current graph mutation epoch. A query-cache entry is valid only while this
	/// equals the epoch captured when the entry was stored.
	pub fn mutation_epoch(&self) -> u64 {
		self.mutation_epoch
	}

	/// The dense entity adjacency for this graph, cached across queries: rebuilt
	/// only when the `mutation_epoch` has moved since the cached copy was built.
	/// Reads take `&self`, so between writes concurrent readers share one Arc.
	pub fn entity_adjacency(&self) -> Arc<EntityAdjacency> {
		let epoch = self.mutation_epoch;
		{
			let cached = crate::base::locks::read_recovered(&self.adjacency_cache);
			if let Some((e, adj)) = cached.as_ref() {
				if *e == epoch {
					return adj.clone();
				}
			}
		}
		let adj = Arc::new(EntityAdjacency::build(self));
		*crate::base::locks::write_recovered(&self.adjacency_cache) = Some((epoch, adj.clone()));
		adj
	}

	pub fn register(&mut self, kern: Kern) {
		let kid = kern.id.clone();
		for t in kern.entities.values() {
			self.entity_kern.insert(t.id.clone(), kid.clone());
		}
		for r in kern.reasons.values() {
			self.reason_kern.insert(r.id.clone(), kid.clone());
		}
		self.unloaded.remove(&kid);
		self.bump_mutation_epoch();
		self.kerns.insert(kid, kern);
		self.enforce_kern_cap();
	}

	pub fn index_entity(&mut self, entity_id: &str, kern_id: &str) {
		self
			.entity_kern
			.insert(entity_id.to_string(), kern_id.to_string());
	}

	pub fn unindex_entity(&mut self, entity_id: &str) {
		self.entity_kern.remove(entity_id);
	}

	pub fn index_reason(&mut self, reason_id: &str, kern_id: &str) {
		self
			.reason_kern
			.insert(reason_id.to_string(), kern_id.to_string());
	}

	pub fn unindex_reason(&mut self, reason_id: &str) {
		self.reason_kern.remove(reason_id);
	}

	pub fn kern_of_entity(&self, entity_id: &str) -> Option<&str> {
		self.entity_kern.get(entity_id).map(|s| s.as_str())
	}

	pub fn kern_of_reason(&self, reason_id: &str) -> Option<&str> {
		self.reason_kern.get(reason_id).map(|s| s.as_str())
	}

	pub fn kern_of_source(&self, external_id: &str) -> Option<&str> {
		self.src_index.get(external_id).map(|s| s.as_str())
	}

	pub fn set_source_entry(&mut self, external_id: String, kern_id: String) {
		self.src_index.insert(external_id, kern_id);
	}

	pub fn delete_source_entry(&mut self, external_id: &str) {
		self.src_index.remove(external_id);
	}

	pub fn loaded(&self, id: &str) -> Option<&Kern> {
		self.kerns.get(id)
	}

	pub fn count(&self) -> usize {
		self.kerns.len() + self.unloaded.len()
	}

	pub fn deregister(&mut self, id: &str) {
		if let Some(kern) = self.kerns.get(id) {
			for tid in kern.entities.keys() {
				self.entity_kern.remove(tid);
			}
			for rid in kern.reasons.keys() {
				self.reason_kern.remove(rid);
			}
		}
		self.kerns.remove(id);
		self.unloaded.remove(id);
		// Removal is a mutation too — flush the cache.
		self.bump_mutation_epoch();
		// Delete the on-disk row too, so a deregistered kern does not resurrect on
		// the next load. (The old file-shard tier needed this because `load_dir`
		// read every `*.kern` as live; the store reconciles on `save_all`, but an
		// explicit delete keeps disk and memory in step immediately.)
		if let Some(store) = &self.store {
			let _ = store.delete_one_kern(id);
		}
	}

	pub fn unload(&mut self, id: &str) -> Result<(), StoreError> {
		if id == self.root.id || !self.kerns.contains_key(id) {
			return Ok(());
		}
		if let Some(store) = self.store.clone() {
			if let Some(k) = self.kerns.get(id) {
				store.save_one_kern(k)?;
			}
		}
		self.kerns.remove(id);
		self.unloaded.insert(id.to_string());
		Ok(())
	}

	/// Reap unnamed kerns that hold no entities and no surviving children — the
	/// residue of the historical unnamed-child spawn runaway (see
	/// [`crate::base::accept::get_or_spawn_unnamed_child`]) which fragments the
	/// graph to `max_kerns` near-empty kerns. Every retrieval seed, tick
	/// `enqueue_all`, and `/graph` render is O(loaded kerns), so this bloat is a
	/// flat tax on latency. Iterates leaf-first until stable, so an empty parent
	/// of (now-removed) empty children is reaped in a later pass. Detaches each
	/// victim from its parent's `children` first, leaving no dangling ref. The
	/// root, named kerns, and any kern with entities or a non-empty descendant are
	/// never touched. Returns the number removed.
	pub fn gc_empty_kerns(&mut self) -> usize {
		let root_id = self.root.id.clone();

		// Liveness reap. A kern is a *seed* of liveness if it is the root, is
		// named (an anchor), or holds at least one entity. Liveness then flows UP
		// the parent chain: every ancestor of a live kern is live, so the path
		// from the root down to real data always survives. Every kern NOT marked
		// live is pure structural residue and is reaped — even if it still has
		// children.
		//
		// Dropping the old `children.is_empty()` leaf-first requirement is the
		// whole point: the unnamed-child spawn runaway produced a *cyclic* forest
		// of empty kerns where every node has children and no childless leaf ever
		// exists, so the leaf-first reap could never start and left hundreds of
		// thousands of empty shards on disk. The parent-walk is cycle-safe via the
		// `live` visited-set: re-encountering an already-live id stops the walk.
		let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
		for k in self.kerns.values() {
			if k.id != root_id && !k.is_named() && k.entities.is_empty() {
				continue;
			}
			let mut cur = k.id.clone();
			loop {
				if !live.insert(cur.clone()) {
					break; // already live → its ancestors are already marked
				}
				let parent = match self.kerns.get(&cur) {
					Some(pk) => pk.parent.clone(),
					None => break,
				};
				if parent.is_empty() || parent == cur {
					break; // reached root or a self-parent guard
				}
				cur = parent;
			}
		}
		live.insert(root_id.clone());

		let victims: std::collections::HashSet<String> = self
			.kerns
			.keys()
			.filter(|id| !live.contains(*id))
			.cloned()
			.collect();
		if victims.is_empty() {
			return 0;
		}

		let removed = victims.len();
		for id in &victims {
			self.deregister(id);
		}

		// Drop every child ref pointing at a kern that no longer exists — the
		// reaped victims AND any file deleted out-of-band. One linear pass over
		// surviving children, keyed on a membership set: O(total children), not
		// O(victims × children), which would explode when the root holds hundreds
		// of thousands of dead child refs (the exact bloat this reaps).
		let existing: std::collections::HashSet<String> = self.kerns.keys().cloned().collect();
		for k in self.kerns.values_mut() {
			if !k.children.is_empty() {
				k.children.retain(|c| existing.contains(c));
			}
		}
		removed
	}

	/// [`gc_empty_kerns`](Self::gc_empty_kerns) wrapped with the loaded-kern counts
	/// either side, as `(before, reaped, after)` — the shape both the startup reap
	/// and the offline `gc` command log.
	pub fn gc_empty_kerns_counted(&mut self) -> (usize, usize, usize) {
		let before = self.kerns.len();
		let reaped = self.gc_empty_kerns();
		(before, reaped, self.kerns.len())
	}

	pub fn all(&self) -> Vec<&Kern> {
		self.kerns.values().collect()
	}

	pub fn all_ids(&self) -> Vec<String> {
		let mut ids: Vec<String> = self.kerns.keys().cloned().collect();
		ids.extend(self.unloaded.iter().cloned());
		ids
	}

	pub fn map(&self) -> &HashMap<String, Kern> {
		&self.kerns
	}

	pub fn unloaded_ids(&self) -> Vec<String> {
		self.unloaded.iter().cloned().collect()
	}

	pub fn from_saved_with_mode(
		root: Kern,
		network_id: String,
		data_dir: String,
		kerns: HashMap<String, Kern>,
		unloaded: HashSet<String>,
		quant_mode: QuantizationMode,
	) -> Self {
		let mut g = Self {
			root: root.clone(),
			network_id,
			data_dir,
			store: None,
			quant_mode,
			entity_idx: VectorBackend::resident(16, 200, quant_mode),
			gnn_entity_idx: VectorBackend::resident(16, 200, quant_mode),
			reason_idx: VectorBackend::resident(16, 200, quant_mode),
			kerns,
			unloaded,
			src_index: HashMap::new(),
			entity_kern: HashMap::new(),
			reason_kern: HashMap::new(),
			lexical: Some(Arc::new(LexicalIndex::new_in_ram(1.2, 0.75))),
			max_loaded_kerns: KERN_CAP_DISABLED,
			disk_threshold: KERN_CAP_DISABLED,
			mutation_epoch: 0,
			flushed_epoch: 0,
			adjacency_cache: parking_lot::RwLock::new(None),
		};
		g.rebuild_index();
		if let Some(lex) = g.lexical.clone() {
			lex.rebuild_from_graph(&g);
		}
		g
	}
}

pub fn migrate_root_id(k: &mut Kern, network_id: &str) {
	if k.root_id.is_empty() {
		k.root_id = network_id.to_string();
	}
	for t in k.entities.values_mut() {
		if t.root_id.is_empty() {
			t.root_id = network_id.to_string();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::Entity;

	fn empty_unnamed(id: &str, parent: &str, children: &[&str]) -> Kern {
		let mut k = Kern::new(id, parent);
		k.children = children.iter().map(|s| s.to_string()).collect();
		k
	}

	#[test]
	fn rebuild_index_excludes_superseded_entities() {
		// Superseded entities are always filtered from retrieval, so re-indexing them
		// on load/rebuild only pollutes candidate generation. rebuild_index must skip
		// them while keeping active entities searchable.
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		if let Some(k) = g.get_mut(&kid) {
			k.entities.insert(
				"active".into(),
				Entity {
					id: "active".into(),
					vector: vec![1.0, 0.0],
					status: EntityStatus::Active,
					..Default::default()
				},
			);
			k.entities.insert(
				"dead".into(),
				Entity {
					id: "dead".into(),
					vector: vec![1.0, 0.0],
					status: EntityStatus::Superseded,
					..Default::default()
				},
			);
		}
		g.rebuild_index();
		let hits: Vec<String> = crate::base::search::search_all_unlocked(&g, &[1.0, 0.0], 5)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert!(
			hits.contains(&"active".to_string()),
			"active entity is indexed"
		);
		assert!(
			!hits.contains(&"dead".to_string()),
			"superseded entity excluded from rebuilt index"
		);
	}

	#[test]
	fn disk_index_snapshot_mirrors_in_ram_membership_and_ranking() {
		// I2: build_entity_disk_index must snapshot EXACTLY what entity_idx holds —
		// active+vector-bearing entities, Superseded excluded — and a DiskIndex
		// opened from it must rank consistently with the in-RAM HNSW (the snapshot
		// is a faithful disk-resident substitute, the basis for later search
		// routing). Vectors are well-separated (distinct per-dim frequencies) so the
		// nearest-neighbour structure is unambiguous despite int8 quant noise in the
		// in-RAM index vs raw f32 on disk.
		use crate::base::diskann::DiskIndex;
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		let vec_of = |i: usize| -> Vec<f32> {
			(0..8)
				.map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin() as f32)
				.collect()
		};
		if let Some(k) = g.get_mut(&kid) {
			for i in 0..80 {
				k.entities.insert(
					format!("e{i}"),
					Entity {
						id: format!("e{i}"),
						vector: vec_of(i),
						status: EntityStatus::Active,
						..Default::default()
					},
				);
			}
			// Superseded entity must never enter the snapshot.
			k.entities.insert(
				"dead".into(),
				Entity {
					id: "dead".into(),
					vector: vec_of(3),
					status: EntityStatus::Superseded,
					..Default::default()
				},
			);
		}
		g.rebuild_index();

		let dir = tempfile::tempdir().unwrap();
		let written = g.build_entity_disk_index(dir.path()).unwrap();
		assert_eq!(
			written, 80,
			"snapshot holds all 80 active entities; superseded excluded"
		);

		let disk = DiskIndex::open(dir.path()).unwrap();
		let q32 = vec_of(40);

		let ram: Vec<String> = crate::base::search::search_all_unlocked(&g, &q32, 10)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		let disk_hits: Vec<String> = disk
			.search_hits(&q32, 10, 96)
			.into_iter()
			.map(|h| h.id)
			.collect();

		assert_eq!(
			disk_hits.first().map(String::as_str),
			Some("e40"),
			"indexed query point ranks first on disk"
		);
		assert_eq!(
			ram.first().map(String::as_str),
			Some("e40"),
			"indexed query point ranks first in RAM"
		);
		assert!(
			!disk_hits.contains(&"dead".to_string()),
			"superseded entity absent from disk snapshot"
		);

		let ram_set: std::collections::HashSet<&String> = ram.iter().collect();
		let overlap = disk_hits.iter().filter(|id| ram_set.contains(id)).count();
		assert!(
			overlap >= 6,
			"disk vs in-RAM top-10 overlap too low: {overlap}/10 (ram={ram:?} disk={disk_hits:?})"
		);
	}

	fn vec8(i: usize) -> Vec<f32> {
		(0..8)
			.map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin() as f32)
			.collect()
	}

	#[test]
	fn rebuild_index_spills_entity_index_to_disk_above_threshold() {
		// I5: rebuild_index routes the entity index to a disk DiskANN snapshot once
		// the resident searchable-entity count crosses disk_threshold, and search
		// keeps working through it. gnn/reason stay in-RAM (entity-only spill here).
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		let kid = g.root.id.clone();
		if let Some(k) = g.get_mut(&kid) {
			for i in 0..40 {
				k.entities.insert(
					format!("e{i}"),
					Entity {
						id: format!("e{i}"),
						vector: vec8(i),
						status: EntityStatus::Active,
						..Default::default()
					},
				);
			}
		}

		// Default threshold (disabled): in-RAM index.
		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Resident(_)),
			"default threshold keeps the in-RAM index"
		);

		// Above threshold: spill to disk, and the snapshot files must exist.
		g.set_disk_threshold(10);
		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Disk { .. }),
			"entity index spilled to disk above threshold"
		);
		assert!(
			dir
				.path()
				.join("diskann")
				.join("entity")
				.join("meta.bin")
				.exists(),
			"on-disk snapshot written"
		);
		// gnn/reason remain resident in this increment.
		assert!(matches!(g.gnn_entity_idx, VectorBackend::Resident(_)));
		assert!(matches!(g.reason_idx, VectorBackend::Resident(_)));

		// Parity across the boundary: an indexed query point ranks itself first via
		// the disk-backed path, and the reverse entity_kern map is still populated.
		let hits = crate::base::search::search_all_unlocked(&g, &vec8(7), 5);
		assert_eq!(
			hits.first().map(|h| h.entity_id.clone()),
			Some("e7".into()),
			"disk-backed search returns the query point first"
		);
		assert!(
			g.kern_of_entity("e7").is_some(),
			"reverse map populated despite skipped entity insert"
		);
	}

	#[test]
	fn rebuild_index_never_spills_without_a_data_dir() {
		// In-memory graph (empty data_dir): there is nowhere to write a snapshot, so
		// the entity index must stay resident even below a tiny threshold.
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		if let Some(k) = g.get_mut(&kid) {
			for i in 0..20 {
				k.entities.insert(
					format!("e{i}"),
					Entity {
						id: format!("e{i}"),
						vector: vec8(i),
						status: EntityStatus::Active,
						..Default::default()
					},
				);
			}
		}
		g.set_disk_threshold(1);
		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Resident(_)),
			"no data_dir -> never spill (nowhere to write)"
		);
	}

	#[test]
	fn consolidate_folds_delta_into_snapshot_and_resets_it() {
		// I6: post-snapshot writes buffer in the delta; consolidate re-snapshots
		// from kerns, leaving the delta empty while keeping every entity searchable.
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		let kid = g.root.id.clone();
		if let Some(k) = g.get_mut(&kid) {
			for i in 0..30 {
				k.entities.insert(
					format!("e{i}"),
					Entity {
						id: format!("e{i}"),
						vector: vec8(i),
						status: EntityStatus::Active,
						..Default::default()
					},
				);
			}
		}
		g.set_disk_threshold(10);
		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Disk { .. }),
			"spilled to disk"
		);
		assert_eq!(
			g.pending_disk_delta_len(),
			0,
			"fresh snapshot has an empty delta"
		);

		// Add entities AFTER the snapshot, mirroring the live path (source of truth
		// AND the index/delta both get the write).
		if let Some(k) = g.get_mut(&kid) {
			for i in 100..115 {
				k.entities.insert(
					format!("e{i}"),
					Entity {
						id: format!("e{i}"),
						vector: vec8(i),
						status: EntityStatus::Active,
						..Default::default()
					},
				);
			}
		}
		for i in 100..115 {
			g.entity_idx.insert(format!("e{i}"), vec8(i));
		}
		assert_eq!(
			g.pending_disk_delta_len(),
			15,
			"post-snapshot inserts buffered in the delta"
		);

		g.consolidate_disk_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Disk { .. }),
			"still disk-backed after consolidate"
		);
		assert_eq!(
			g.pending_disk_delta_len(),
			0,
			"delta folded into the rebuilt snapshot"
		);

		// Both pre- and post-snapshot entities are searchable from the new snapshot.
		let new_hit = crate::base::search::search_all_unlocked(&g, &vec8(108), 5);
		assert_eq!(
			new_hit.first().map(|h| h.entity_id.clone()),
			Some("e108".into()),
			"folded-in entity searchable"
		);
		let old_hit = crate::base::search::search_all_unlocked(&g, &vec8(5), 5);
		assert_eq!(
			old_hit.first().map(|h| h.entity_id.clone()),
			Some("e5".into()),
			"original entity still searchable"
		);
	}

	#[test]
	fn consolidate_is_a_noop_for_a_resident_index() {
		// Never disk-backed: consolidate must not panic or change the backend.
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		if let Some(k) = g.get_mut(&kid) {
			k.entities.insert(
				"a".into(),
				Entity {
					id: "a".into(),
					vector: vec8(1),
					status: EntityStatus::Active,
					..Default::default()
				},
			);
		}
		g.rebuild_index();
		g.consolidate_disk_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Resident(_)),
			"resident index untouched"
		);
		assert_eq!(g.pending_disk_delta_len(), 0);
	}

	#[test]
	fn gc_reaps_cyclic_empty_kerns_with_children() {
		// Reproduces the spawn-runaway shape: a cycle of empty unnamed kerns where
		// every node has children and NO childless leaf exists. The old leaf-first
		// reap could not start here and left them on disk forever.
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();

		// Two empty unnamed kerns pointing at each other → a 2-cycle, no leaf.
		g.register(empty_unnamed("A", &root_id, &["B"]));
		g.register(empty_unnamed("B", "A", &["A"]));

		// A named anchor (kept) and an entity-bearing kern (kept), both under root.
		let mut named = Kern::new("N", &root_id);
		named.anchor_text = "durable facts".into();
		g.register(named);

		let mut withent = Kern::new("E", &root_id);
		withent.entities.insert(
			"e1".into(),
			Entity {
				id: "e1".into(),
				..Default::default()
			},
		);
		g.register(withent);

		// Root references all four children (mirrors the real on-disk root).
		if let Some(r) = g.kerns.get_mut(&root_id) {
			r.children = vec!["A".into(), "B".into(), "N".into(), "E".into()];
		}

		let before = g.kerns.len();
		let reaped = g.gc_empty_kerns();

		assert_eq!(
			reaped, 2,
			"both cyclic empty kerns reaped despite having children"
		);
		assert!(g.loaded("A").is_none(), "A reaped");
		assert!(g.loaded("B").is_none(), "B reaped");
		assert!(g.loaded("N").is_some(), "named anchor kept");
		assert!(g.loaded("E").is_some(), "entity-bearing kern kept");
		assert!(g.loaded(&root_id).is_some(), "root kept");
		assert_eq!(g.kerns.len(), before - 2);

		// Root's child list no longer references the reaped kerns.
		let root_children = &g.kerns.get(&root_id).unwrap().children;
		assert!(
			!root_children.contains(&"A".to_string()),
			"dead ref A scrubbed"
		);
		assert!(
			!root_children.contains(&"B".to_string()),
			"dead ref B scrubbed"
		);
		assert!(root_children.contains(&"N".to_string()) && root_children.contains(&"E".to_string()));
	}

	#[test]
	fn gc_keeps_empty_ancestor_on_path_to_data() {
		// An empty unnamed kern that sits BETWEEN root and an entity-bearing kern
		// must survive — it is the only path to real data. Liveness flows up.
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();

		g.register(empty_unnamed("mid", &root_id, &["leaf"]));
		let mut leaf = Kern::new("leaf", "mid");
		leaf.entities.insert(
			"e1".into(),
			Entity {
				id: "e1".into(),
				..Default::default()
			},
		);
		g.register(leaf);
		if let Some(r) = g.kerns.get_mut(&root_id) {
			r.children = vec!["mid".into()];
		}

		let reaped = g.gc_empty_kerns();
		assert_eq!(reaped, 0, "empty ancestor of data is not reaped");
		assert!(g.loaded("mid").is_some(), "ancestor on path to data kept");
		assert!(g.loaded("leaf").is_some(), "data kern kept");
	}
}
