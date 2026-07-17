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

#[allow(clippy::too_many_arguments)]
fn index_kern_into(
	kern: &Kern,
	entity_kern: &mut HashMap<String, String>,
	reason_kern: &mut HashMap<String, String>,
	src_index: &mut HashMap<String, String>,
	// `None` skips entity-vector inserts: a disk snapshot ALREADY holds every
	// resident entity — re-inserting would tombstone it all into the delta.
	mut entity_idx: Option<&mut VectorBackend>,
	gnn_entity_idx: &mut VectorBackend,
	reason_idx: &mut VectorBackend,
) {
	// HNSW structure depends on insert order — populate in id order, never HashMap
	// order (differs per process).
	let mut entities: Vec<_> = kern.entities.values().collect();
	entities.sort_by(|a, b| a.id.cmp(&b.id));
	for t in entities {
		entity_kern.insert(t.id.clone(), kern.id.clone());
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
	let mut reasons: Vec<_> = kern.reasons.values().collect();
	reasons.sort_by(|a, b| a.id.cmp(&b.id));
	for r in reasons {
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
	// LMDB forbids opening one env twice in a process; opened once and shared.
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
	max_loaded_kerns: usize,
	disk_threshold: usize,
	// Must stay GLOBAL — per-kern versions are unsound under HyDE.
	mutation_epoch: u64,
	flushed_epoch: u64,
	adjacency_cache: parking_lot::RwLock<Option<(u64, Arc<EntityAdjacency>)>>,
}

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
		Self {
			id_to_idx,
			ids,
			out,
		}
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

	pub fn flushed_epoch(&self) -> u64 {
		self.flushed_epoch
	}

	// Not a content mutation — must NOT bump mutation_epoch.
	pub fn set_flushed_epoch(&mut self, epoch: u64) {
		self.flushed_epoch = epoch;
	}

	pub fn set_max_loaded_kerns(&mut self, cap: usize) {
		self.max_loaded_kerns = cap.max(1);
	}

	pub fn set_disk_threshold(&mut self, threshold: usize) {
		self.disk_threshold = threshold;
	}

	pub fn set_store(&mut self, store: Arc<Store>) {
		self.store = Some(store);
	}

	pub fn store(&self) -> Option<Arc<Store>> {
		self.store.clone()
	}

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

		let entity_count = self.resident_searchable_entity_count();
		let spill = !self.data_dir.is_empty() && entity_count > self.disk_threshold;
		self.entity_idx = match spill.then(|| self.build_entity_disk_snapshot()).flatten() {
			Some(snapshot) => VectorBackend::disk(snapshot, self.quant_mode),
			None => VectorBackend::resident(16, 200, self.quant_mode),
		};

		// A disk snapshot already holds every resident entity — `None` skips the
		// re-insert (which would tombstone it) but still fills the reverse maps.
		let skip_entity_insert = matches!(self.entity_idx, VectorBackend::Disk { .. });
		let mut kerns: Vec<&Kern> = self.kerns.values().collect();
		kerns.sort_by(|a, b| a.id.cmp(&b.id));
		for kern in kerns {
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

	// Filter must mirror index_kern_into (drives the spill decision).
	fn resident_searchable_entity_count(&self) -> usize {
		self
			.kerns
			.values()
			.flat_map(|k| k.entities.values())
			.filter(|t| t.status != EntityStatus::Superseded && t.has_vector())
			.count()
	}

	// id-sorted (BTreeMap) so the Vamana build is reproducible.
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

	pub fn build_entity_disk_index(&self, dir: &std::path::Path) -> std::io::Result<usize> {
		super::diskann::build_and_save(
			dir,
			&self.collect_entity_items(),
			super::diskann::Params::default(),
		)
	}

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

	// COST: the Vamana build runs under the graph WRITE lock.
	pub fn consolidate_disk_index(&mut self) {
		if !matches!(self.entity_idx, VectorBackend::Disk { .. }) {
			return;
		}
		// Drop the old mmap FIRST so the rebuild can overwrite its files (Windows
		// locks mmapped files).
		self.entity_idx = VectorBackend::resident(16, 200, self.quant_mode);
		match self.build_entity_disk_snapshot() {
			Some(snapshot) => self.entity_idx = VectorBackend::disk(snapshot, self.quant_mode),
			None => self.rebuild_index(),
		}
	}

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
			self.bump_mutation_epoch();
		}
		if let Some(k) = self.kerns.get_mut(id) {
			k.last_access = Some(SystemTime::now());
			Some(k)
		} else {
			None
		}
	}

	pub fn bump_mutation_epoch(&mut self) {
		self.mutation_epoch = self.mutation_epoch.wrapping_add(1);
	}

	pub fn mutation_epoch(&self) -> u64 {
		self.mutation_epoch
	}

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
		self.bump_mutation_epoch();
		// Delete the on-disk row so a deregistered kern does not resurrect on load.
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

	pub fn gc_empty_kerns(&mut self) -> usize {
		let root_id = self.root.id.clone();

		// Cycle-safe via the `live` visited-set: re-encountering a live id stops.
		let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
		for k in self.kerns.values() {
			if k.id != root_id && !k.is_named() && k.entities.is_empty() {
				continue;
			}
			let mut cur = k.id.clone();
			loop {
				if !live.insert(cur.clone()) {
					break;
				}
				let parent = match self.kerns.get(&cur) {
					Some(pk) => pk.parent.clone(),
					None => break,
				};
				if parent.is_empty() || parent == cur {
					break;
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

		let existing: std::collections::HashSet<String> = self.kerns.keys().cloned().collect();
		for k in self.kerns.values_mut() {
			if !k.children.is_empty() {
				k.children.retain(|c| existing.contains(c));
			}
		}
		removed
	}

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
	fn rebuild_index_is_deterministic_across_instances() {
		use crate::base::types::Reason;
		let vec_of = |i: usize, off: f64| -> Vec<f32> {
			(0..8)
				.map(|j| ((i as f64) * (0.11 + 0.05 * j as f64) + off).sin() as f32)
				.collect()
		};
		let make_kern = |k: usize| -> Kern {
			let mut kern = Kern::new(format!("k{k}"), "root");
			for e in 0..40 {
				let id = format!("k{k}e{e}");
				kern.entities.insert(
					id.clone(),
					Entity {
						id,
						vector: vec_of(k * 100 + e, 0.0),
						gnn_vector: vec_of(k * 100 + e, 0.5),
						..Default::default()
					},
				);
			}
			for r in 0..10 {
				let id = format!("k{k}r{r}");
				kern.reasons.insert(
					id.clone(),
					Reason {
						id,
						vector: vec_of(k * 100 + r, 1.0),
						..Default::default()
					},
				);
			}
			kern
		};
		let digest = |be: &VectorBackend| match be {
			VectorBackend::Resident(h) => h.structure_digest(),
			VectorBackend::Disk { .. } => unreachable!("test graphs never spill"),
		};
		let mut a = GraphGnn::new();
		for k in 0..5 {
			let kern = make_kern(k);
			a.kerns.insert(kern.id.clone(), kern);
		}
		let mut b = GraphGnn::new();
		for k in (0..5).rev() {
			let kern = make_kern(k);
			b.kerns.insert(kern.id.clone(), kern);
		}
		a.rebuild_index();
		b.rebuild_index();
		assert_eq!(
			digest(&a.entity_idx),
			digest(&b.entity_idx),
			"entity index structure differs across instances"
		);
		assert_eq!(
			digest(&a.gnn_entity_idx),
			digest(&b.gnn_entity_idx),
			"gnn index structure differs across instances"
		);
		assert_eq!(
			digest(&a.reason_idx),
			digest(&b.reason_idx),
			"reason index structure differs across instances"
		);
	}

	#[test]
	fn rebuild_index_excludes_superseded_entities() {
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
		// Vectors use distinct per-dim frequencies so the nearest-neighbour structure
		// is unambiguous despite in-RAM int8 quant noise vs raw f32 on disk.
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

		g.rebuild_index();
		assert!(
			matches!(g.entity_idx, VectorBackend::Resident(_)),
			"default threshold keeps the in-RAM index"
		);

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
		assert!(matches!(g.gnn_entity_idx, VectorBackend::Resident(_)));
		assert!(matches!(g.reason_idx, VectorBackend::Resident(_)));

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

		// Mirror the live path: source of truth AND the index/delta both get the write.
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
		// The spawn-runaway shape: a cycle of empty kerns with NO childless leaf —
		// do NOT simplify to a leaf-first reap, which can never start here.
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();

		g.register(empty_unnamed("A", &root_id, &["B"]));
		g.register(empty_unnamed("B", "A", &["A"]));

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
