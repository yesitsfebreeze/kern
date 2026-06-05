use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use super::hnsw::HnswIndex;
use super::lexical::LexicalIndex;
use super::types::Kern;
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
	entity_idx: &mut HnswIndex,
	gnn_entity_idx: &mut HnswIndex,
	reason_idx: &mut HnswIndex,
) {
	for t in kern.entities.values() {
		entity_kern.insert(t.id.clone(), kern.id.clone());
		if t.has_vector() {
			entity_idx.insert(t.id.clone(), t.vector.clone());
		}
		if t.has_gnn_vector() {
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
	pub quant_mode: QuantizationMode,
	pub gnn_entity_idx: HnswIndex,
	pub entity_idx: HnswIndex,
	pub reason_idx: HnswIndex,
	pub kerns: HashMap<String, Kern>,
	unloaded: HashSet<String>,
	src_index: HashMap<String, String>,
	entity_kern: HashMap<String, String>,
	reason_kern: HashMap<String, String>,
	lexical: Option<Arc<LexicalIndex>>,
	/// Soft cap on the number of kerns held in memory at once. When `register`
	/// would exceed this, the LRU (oldest `last_access`) non-root kern is
	/// `unload`ed to disk. `usize::MAX` disables the cap.
	max_loaded_kerns: usize,
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
			quant_mode,
			entity_idx: HnswIndex::with_mode(16, 200, quant_mode),
			gnn_entity_idx: HnswIndex::with_mode(16, 200, quant_mode),
			reason_idx: HnswIndex::with_mode(16, 200, quant_mode),
			kerns,
			unloaded: HashSet::new(),
			src_index: HashMap::new(),
			entity_kern: HashMap::new(),
			reason_kern: HashMap::new(),
			lexical: Some(Arc::new(LexicalIndex::new_in_ram(1.2, 0.75))),
			max_loaded_kerns: usize::MAX,
		}
	}

	pub fn set_max_loaded_kerns(&mut self, cap: usize) {
		self.max_loaded_kerns = cap.max(1);
	}

	/// Evict the oldest non-root kern by `last_access` while we are over the
	/// soft cap. Errors during `unload` (persist failures) are swallowed —
	/// the caller already accepted that we may degrade under pressure.
	fn enforce_kern_cap(&mut self) {
		if self.max_loaded_kerns == usize::MAX {
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
		self.entity_idx = HnswIndex::with_mode(16, 200, self.quant_mode);
		self.gnn_entity_idx = HnswIndex::with_mode(16, 200, self.quant_mode);
		self.reason_idx = HnswIndex::with_mode(16, 200, self.quant_mode);
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

	pub fn get(&mut self, id: &str) -> Option<&Kern> {
		if self.kerns.contains_key(id) {
			if let Some(k) = self.kerns.get_mut(id) {
				k.last_access = Some(SystemTime::now());
			}
			return self.kerns.get(id);
		}
		if !self.data_dir.is_empty() && self.unloaded.contains(id) {
			if let Ok(mut k) = super::persist::load_kern(&self.data_dir, id) {
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
		if let Some(k) = self.kerns.get_mut(id) {
			k.last_access = Some(SystemTime::now());
			Some(k)
		} else {
			None
		}
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
	}

	pub fn unload(&mut self, id: &str) -> Result<(), super::persist::PersistError> {
		if id == self.root.id || !self.kerns.contains_key(id) {
			return Ok(());
		}
		if !self.data_dir.is_empty() {
			if let Some(k) = self.kerns.get(id) {
				super::persist::save_kern(&self.data_dir, k)?;
			}
		}
		self.kerns.remove(id);
		self.unloaded.insert(id.to_string());
		Ok(())
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

	pub fn from_saved(
		root: Kern,
		network_id: String,
		data_dir: String,
		kerns: HashMap<String, Kern>,
		unloaded: HashSet<String>,
	) -> Self {
		Self::from_saved_with_mode(
			root,
			network_id,
			data_dir,
			kerns,
			unloaded,
			QuantizationMode::None,
		)
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
			quant_mode,
			entity_idx: HnswIndex::with_mode(16, 200, quant_mode),
			gnn_entity_idx: HnswIndex::with_mode(16, 200, quant_mode),
			reason_idx: HnswIndex::with_mode(16, 200, quant_mode),
			kerns,
			unloaded,
			src_index: HashMap::new(),
			entity_kern: HashMap::new(),
			reason_kern: HashMap::new(),
			lexical: Some(Arc::new(LexicalIndex::new_in_ram(1.2, 0.75))),
			max_loaded_kerns: usize::MAX,
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
