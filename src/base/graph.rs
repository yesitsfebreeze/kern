use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use super::constants::KERN_CAP_DISABLED;
use super::lexical::LexicalIndex;
use super::vector_backend::VectorBackend;
use super::store::{Store, StoreError};
use super::types::{EntityStatus, Kern};
use super::util;
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
	entity_idx: &mut VectorBackend,
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
			entity_idx.insert(t.id.clone(), t.vector.clone());
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
			mutation_epoch: 0,
		}
	}

	pub fn set_max_loaded_kerns(&mut self, cap: usize) {
		self.max_loaded_kerns = cap.max(1);
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
		self.entity_idx = VectorBackend::resident(16, 200, self.quant_mode);
		self.gnn_entity_idx = VectorBackend::resident(16, 200, self.quant_mode);
		self.reason_idx = VectorBackend::resident(16, 200, self.quant_mode);
		self.src_index.clear();
		self.entity_kern.clear();
		self.reason_kern.clear();
		for kern in self.kerns.values() {
			index_kern_into(
				kern,
				&mut self.entity_kern,
				&mut self.reason_kern,
				&mut self.src_index,
				&mut self.entity_idx,
				&mut self.gnn_entity_idx,
				&mut self.reason_idx,
			);
		}
	}

	/// Snapshot every searchable resident entity's content vector into a
	/// disk-resident Vamana index under `dir`, mirroring EXACTLY the membership
	/// of [`entity_idx`](Self::entity_idx): Superseded entities are skipped (same
	/// rule as [`index_kern_into`]) and vector-less entities are skipped. Entities
	/// are keyed and emitted in id order, so a `f64`-snapshot of a fixed resident
	/// set yields a byte-reproducible index (DiskANN's build is seeded). Vectors
	/// are narrowed `f64 -> f32` at the boundary — lossy but consistent with the
	/// int8-on-disk storage posture; ANN recall is unaffected in practice.
	///
	/// This is the BUILD half of the DiskANN integration (increment I2 of
	/// `docs/superpowers/plans/2026-06-12-diskann-wiring.md`). Search routing
	/// through the snapshot lands in a later increment; this method has no effect
	/// on the live search path. Returns the number of vectors written.
	pub fn build_entity_disk_index(&self, dir: &std::path::Path) -> std::io::Result<usize> {
		// BTreeMap: dedup by id (mirroring the id-keyed HnswIndex) AND deterministic
		// id ordering so the seeded Vamana build is reproducible across runs.
		let mut items: std::collections::BTreeMap<String, Vec<f32>> =
			std::collections::BTreeMap::new();
		for kern in self.kerns.values() {
			for t in kern.entities.values() {
				if t.status != EntityStatus::Superseded && t.has_vector() {
					items.insert(t.id.clone(), t.vector.iter().map(|&x| x as f32).collect());
				}
			}
		}
		let items: Vec<(String, Vec<f32>)> = items.into_iter().collect();
		super::diskann::build_and_save(dir, &items, super::diskann::Params::default())
	}

	pub fn get(&mut self, id: &str) -> Option<&Kern> {
		if self.kerns.contains_key(id) {
			if let Some(k) = self.kerns.get_mut(id) {
				k.last_access = Some(SystemTime::now());
			}
			return self.kerns.get(id);
		}
		if self.unloaded.contains(id) {
			let loaded = self.store.clone().and_then(|s| s.load_one_kern(id).ok().flatten());
			if let Some(mut k) = loaded {
				migrate_root_id(&mut k, &self.network_id);
				k.last_access = Some(SystemTime::now());
				index_kern_into(
					&k,
					&mut self.entity_kern,
					&mut self.reason_kern,
					&mut self.src_index,
					&mut self.entity_idx,
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

		// Detach victims from EVERY surviving kern's children in one linear pass —
		// not per-victim, which is O(victims × children) and explodes when the
		// root holds hundreds of thousands of dead child refs (the exact bloat
		// this reaps). HashSet membership keeps it O(total children).
		for k in self.kerns.values_mut() {
			if !k.children.is_empty() {
				k.children.retain(|c| !victims.contains(c));
			}
		}
		let mut removed = 0usize;
		for id in &victims {
			self.deregister(id);
			removed += 1;
		}

		// Final hygiene: drop any child ref pointing at a kern that no longer
		// exists in the graph. Covers victims removed above AND files deleted
		// out-of-band, which otherwise leaves a surviving kern carrying dead refs.
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
			mutation_epoch: 0,
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
				Entity { id: "active".into(), vector: vec![1.0, 0.0], status: EntityStatus::Active, ..Default::default() },
			);
			k.entities.insert(
				"dead".into(),
				Entity { id: "dead".into(), vector: vec![1.0, 0.0], status: EntityStatus::Superseded, ..Default::default() },
			);
		}
		g.rebuild_index();
		let hits: Vec<String> = crate::base::search::search_all_unlocked(&g, &[1.0, 0.0], 5)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert!(hits.contains(&"active".to_string()), "active entity is indexed");
		assert!(!hits.contains(&"dead".to_string()), "superseded entity excluded from rebuilt index");
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
		let vec_of = |i: usize| -> Vec<f64> {
			(0..8).map(|j| ((i as f64) * (0.13 + 0.07 * j as f64)).sin()).collect()
		};
		if let Some(k) = g.get_mut(&kid) {
			for i in 0..80 {
				k.entities.insert(
					format!("e{i}"),
					Entity { id: format!("e{i}"), vector: vec_of(i), status: EntityStatus::Active, ..Default::default() },
				);
			}
			// Superseded entity must never enter the snapshot.
			k.entities.insert(
				"dead".into(),
				Entity { id: "dead".into(), vector: vec_of(3), status: EntityStatus::Superseded, ..Default::default() },
			);
		}
		g.rebuild_index();

		let dir = tempfile::tempdir().unwrap();
		let written = g.build_entity_disk_index(dir.path()).unwrap();
		assert_eq!(written, 80, "snapshot holds all 80 active entities; superseded excluded");

		let disk = DiskIndex::open(dir.path()).unwrap();
		let q64 = vec_of(40);
		let q32: Vec<f32> = q64.iter().map(|&x| x as f32).collect();

		let ram: Vec<String> = crate::base::search::search_all_unlocked(&g, &q64, 10)
			.into_iter().map(|h| h.entity_id).collect();
		let disk_hits: Vec<String> = disk.search_hits(&q32, 10, 96).into_iter().map(|h| h.id).collect();

		assert_eq!(disk_hits.first().map(String::as_str), Some("e40"), "indexed query point ranks first on disk");
		assert_eq!(ram.first().map(String::as_str), Some("e40"), "indexed query point ranks first in RAM");
		assert!(!disk_hits.contains(&"dead".to_string()), "superseded entity absent from disk snapshot");

		let ram_set: std::collections::HashSet<&String> = ram.iter().collect();
		let overlap = disk_hits.iter().filter(|id| ram_set.contains(id)).count();
		assert!(overlap >= 6, "disk vs in-RAM top-10 overlap too low: {overlap}/10 (ram={ram:?} disk={disk_hits:?})");
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
		withent.entities.insert("e1".into(), Entity { id: "e1".into(), ..Default::default() });
		g.register(withent);

		// Root references all four children (mirrors the real on-disk root).
		if let Some(r) = g.kerns.get_mut(&root_id) {
			r.children = vec!["A".into(), "B".into(), "N".into(), "E".into()];
		}

		let before = g.kerns.len();
		let reaped = g.gc_empty_kerns();

		assert_eq!(reaped, 2, "both cyclic empty kerns reaped despite having children");
		assert!(g.loaded("A").is_none(), "A reaped");
		assert!(g.loaded("B").is_none(), "B reaped");
		assert!(g.loaded("N").is_some(), "named anchor kept");
		assert!(g.loaded("E").is_some(), "entity-bearing kern kept");
		assert!(g.loaded(&root_id).is_some(), "root kept");
		assert_eq!(g.kerns.len(), before - 2);

		// Root's child list no longer references the reaped kerns.
		let root_children = &g.kerns.get(&root_id).unwrap().children;
		assert!(!root_children.contains(&"A".to_string()), "dead ref A scrubbed");
		assert!(!root_children.contains(&"B".to_string()), "dead ref B scrubbed");
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
		leaf.entities.insert("e1".into(), Entity { id: "e1".into(), ..Default::default() });
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
