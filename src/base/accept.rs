use super::constants::*;
use super::graph::GraphGnn;
use super::math::{average_vec, cosine_distance, reason_id};
use super::reason::add_reason;
use super::search::search_all_unlocked;
use super::types::*;
use crate::crdt::GCounter;

#[derive(Debug)]
pub struct AcceptResult {
	pub placed_in: String,
	pub entity_id: String,
	pub deduped: bool,
	pub reason_ids: Vec<String>,
}

const MAX_ACCEPT_DEPTH: usize = 64;

pub fn accept(g: &mut GraphGnn, kern_id: &str, thought: Entity, doc_id: &str) -> AcceptResult {
	// Dedup scans graph-wide and routing only reads or spawns empty kerns, so
	// the result cannot change during descent — safe to compute once.
	let is_dup = is_duplicate(g, &thought.vector);
	let target_id = route_entity(g, kern_id, &thought, is_dup);
	commit_entity(g, &target_id, thought, doc_id, is_dup)
}

fn is_duplicate(g: &GraphGnn, vector: &[f32]) -> bool {
	let hits = search_all_unlocked(g, vector, 1);
	!hits.is_empty() && hits[0].score > DEFAULT_DEDUP_THRESHOLD
}

fn route_entity(g: &mut GraphGnn, kern_id: &str, thought: &Entity, is_dup: bool) -> String {
	let mut current_id = kern_id.to_string();

	if is_dup {
		return current_id;
	}

	for _depth in 0..MAX_ACCEPT_DEPTH {
		let children = g
			.loaded(&current_id)
			.map(|k| k.children.clone())
			.unwrap_or_default();
		if let Some(child_id) = route_to_child_id(&children, g, &thought.vector) {
			current_id = child_id;
			continue;
		}

		// The root is a pure dispatcher: a no-anchor-match falls through to the
		// `generic` catch-all, never commits onto the root itself.
		if current_id == g.root.id {
			let generic_id = get_or_spawn_generic_child(g, &current_id);
			if generic_id != current_id {
				current_id = generic_id;
				continue;
			}
			break;
		}

		let reject = {
			let kern = match g.loaded(&current_id) {
				Some(k) => k,
				None => break,
			};
			if kern.has_anchor() {
				let dist = cosine_distance(&thought.vector, &kern.anchor_vec);
				let p = acceptance_probability(dist, kern.inner_radius, kern.outer_radius);
				p < ACCEPT_FLOOR
			} else {
				false
			}
		};

		if reject {
			let child_id = get_or_spawn_unnamed_child(g, &current_id);
			current_id = child_id;
			continue;
		}

		break;
	}
	current_id
}

fn commit_entity(
	g: &mut GraphGnn,
	kern_id: &str,
	mut thought: Entity,
	doc_id: &str,
	is_dup: bool,
) -> AcceptResult {
	if is_dup {
		return AcceptResult {
			placed_in: kern_id.to_string(),
			entity_id: thought.id.clone(),
			deduped: true,
			reason_ids: Vec::new(),
		};
	}

	let root_id = g
		.loaded(kern_id)
		.map(|k| k.root_id.clone())
		.unwrap_or_default();
	thought.root_id = root_id;
	let entity_id = thought.id.clone();
	let thought_vec = thought.vector.clone();
	let external_id = thought.external_id.clone();

	if thought.has_vector() {
		g.entity_idx.insert(entity_id.clone(), thought_vec.clone());
	}

	if let Some(kern) = g.get_mut(kern_id) {
		kern.entities.insert(entity_id.clone(), thought);
	}
	g.index_entity(&entity_id, kern_id);

	let mut reason_ids = Vec::new();

	reason_ids.extend(add_similarity_reason(g, kern_id, &entity_id, &thought_vec));

	reason_ids.extend(add_provenance_reason(
		g,
		kern_id,
		&entity_id,
		&thought_vec,
		doc_id,
	));

	if !external_id.is_empty() {
		reason_ids.extend(supersede(
			g,
			kern_id,
			&entity_id,
			&thought_vec,
			&external_id,
		));
	}

	AcceptResult {
		placed_in: kern_id.to_string(),
		entity_id,
		deduped: false,
		reason_ids,
	}
}

fn commit_reason(
	g: &mut GraphGnn,
	kern_id: &str,
	from: &str,
	to: &str,
	kind: ReasonKind,
	score: f64,
	vec: Vec<f32>,
) -> String {
	let rid = reason_id(from, to, kind, "", "");
	let reason = Reason {
		id: rid.clone(),
		from: from.to_string(),
		to: to.to_string(),
		to_kern_id: String::new(),
		to_net_id: String::new(),
		kind,
		dirty: false,
		text: String::new(),
		vector: vec.clone(),
		score,
		traversal_count: GCounter::new(),
		producer_id: String::new(),
	};
	if !vec.is_empty() {
		g.reason_idx.insert(rid.clone(), vec);
	}
	if let Some(kern) = g.get_mut(kern_id) {
		add_reason(kern, reason);
	}
	g.index_reason(&rid, kern_id);
	rid
}

fn add_similarity_reason(
	g: &mut GraphGnn,
	kern_id: &str,
	entity_id: &str,
	thought_vec: &[f32],
) -> Vec<String> {
	let hits = search_all_unlocked(g, thought_vec, 2);
	for h in &hits {
		if h.entity_id == entity_id {
			continue;
		}
		let nearest_vec = g
			.kern_of_entity(&h.entity_id)
			.and_then(|kid| g.loaded(kid))
			.and_then(|kern| kern.entities.get(&h.entity_id))
			.map(|t| t.vector.clone())
			.unwrap_or_default();

		let vec = if !thought_vec.is_empty() && !nearest_vec.is_empty() {
			average_vec(thought_vec, &nearest_vec)
		} else {
			Vec::new()
		};

		let rid = commit_reason(
			g,
			kern_id,
			entity_id,
			&h.entity_id,
			ReasonKind::Similarity,
			h.score,
			vec,
		);
		return vec![rid];
	}
	Vec::new()
}

fn add_provenance_reason(
	g: &mut GraphGnn,
	kern_id: &str,
	entity_id: &str,
	thought_vec: &[f32],
	doc_id: &str,
) -> Vec<String> {
	if doc_id.is_empty() {
		return Vec::new();
	}
	let doc_vec = g
		.loaded(kern_id)
		.and_then(|k| k.entities.get(doc_id))
		.filter(|t| t.has_vector())
		.map(|t| t.vector.clone());

	let vec = match (&doc_vec, thought_vec.is_empty()) {
		(Some(dv), false) => average_vec(thought_vec, dv),
		_ => Vec::new(),
	};

	let rid = commit_reason(
		g,
		kern_id,
		entity_id,
		doc_id,
		ReasonKind::Provenance,
		PROVENANCE_SCORE,
		vec,
	);
	vec![rid]
}

fn supersede(
	g: &mut GraphGnn,
	placed_kern_id: &str,
	entity_id: &str,
	thought_vec: &[f32],
	external_id: &str,
) -> Vec<String> {
	let index_kern_id = g.kern_of_source(external_id).map(|s| s.to_string());
	let old_id = index_kern_id.as_ref().and_then(|kid| {
		g.loaded(kid)
			.and_then(|k| k.source_index.get(external_id).cloned())
	});

	if old_id.as_deref() == Some(entity_id) {
		return Vec::new();
	}

	if let Some(ref ik) = index_kern_id {
		if ik != placed_kern_id {
			if let Some(kern) = g.get_mut(ik) {
				kern.source_index.remove(external_id);
			}
		}
	}
	if let Some(kern) = g.get_mut(placed_kern_id) {
		kern
			.source_index
			.insert(external_id.to_string(), entity_id.to_string());
	}
	g.set_source_entry(external_id.to_string(), placed_kern_id.to_string());

	let old_id = match old_id {
		Some(id) => id,
		None => return Vec::new(),
	};

	let (old_vec, old_kern_id) = {
		let mut found = None;
		if let Some(ref ik) = index_kern_id {
			if let Some(kern) = g.loaded(ik) {
				if let Some(t) = kern.entities.get(&old_id) {
					found = Some((t.vector.clone(), ik.clone()));
				}
			}
		}
		if found.is_none() {
			// `get` auto-loads the owning kern if it was evicted, so this also
			// finds entities a loaded-only scan would miss.
			if let Some(kid) = g.kern_of_entity(&old_id).map(|s| s.to_string()) {
				if let Some(kern) = g.get(&kid) {
					if let Some(t) = kern.entities.get(&old_id) {
						found = Some((t.vector.clone(), kid));
					}
				}
			}
		}
		match found {
			Some(f) => f,
			None => return Vec::new(),
		}
	};

	let now = std::time::SystemTime::now();
	let new_valid_from = g
		.loaded(placed_kern_id)
		.and_then(|k| k.entities.get(entity_id))
		.and_then(|e| e.valid_from_or_created())
		.unwrap_or(now);
	if let Some(kern) = g.get_mut(&old_kern_id) {
		if let Some(old) = kern.entities.get_mut(&old_id) {
			old.status = EntityStatus::Superseded;
			old.superseded_by = entity_id.to_string();
			old.stamp_invalidated(now, new_valid_from);
		}
	}

	// A superseded entity is never a valid retrieval result — evict from the ANN
	// indices; it stays in `kern.entities` so the supersede chain holds.
	g.entity_idx.delete(&old_id);
	g.gnn_entity_idx.delete(&old_id);

	let vec = if !thought_vec.is_empty() && !old_vec.is_empty() {
		average_vec(thought_vec, &old_vec)
	} else {
		Vec::new()
	};

	vec![commit_reason(
		g,
		placed_kern_id,
		entity_id,
		&old_id,
		ReasonKind::Supersedes,
		1.0,
		vec,
	)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContradictionClass {
	Supersede,
	Related,
}

pub fn classify_prompt(old_text: &str, new_text: &str) -> String {
	format!(
		"Two statements are near-duplicates about the same subject. Decide whether \
the NEW statement UPDATES or CONTRADICTS the OLD one (so the new should replace \
the old), or is merely RELATED (both can coexist). Answer with exactly ONE word: \
UPDATE, CONTRADICTION, or RELATED.\n\nOLD: {old_text}\nNEW: {new_text}\n"
	)
}

// Fails open to Related (any RELATED mention wins) — the conservative choice.
pub fn parse_contradiction(raw: &str) -> ContradictionClass {
	let up = raw.trim().to_uppercase();
	let supersede = up.contains("CONTRADICT") || up.contains("UPDATE");
	if supersede && !up.contains("RELATED") {
		ContradictionClass::Supersede
	} else {
		ContradictionClass::Related
	}
}

pub fn supersede_by_contradiction(
	g: &mut GraphGnn,
	kern_id: &str,
	old_id: &str,
	new_thought: Entity,
) -> Vec<String> {
	let new_id = new_thought.id.clone();
	if new_id == old_id {
		return Vec::new();
	}
	let old_kern_id = match g.kern_of_entity(old_id).map(str::to_string) {
		Some(k) => k,
		None => return Vec::new(),
	};
	let (old_vec, already_superseded) =
		match g.loaded(&old_kern_id).and_then(|k| k.entities.get(old_id)) {
			Some(o) => (o.vector.clone(), o.is_superseded()),
			None => return Vec::new(),
		};
	if already_superseded {
		return Vec::new();
	}

	let new_vec = new_thought.vector.clone();
	let new_valid_from = new_thought
		.valid_from_or_created()
		.unwrap_or_else(std::time::SystemTime::now);
	let root_id = g
		.loaded(kern_id)
		.map(|k| k.root_id.clone())
		.unwrap_or_default();

	let mut new_thought = new_thought;
	new_thought.root_id = root_id;
	if new_thought.has_vector() {
		g.entity_idx.insert(new_id.clone(), new_vec.clone());
	}
	if let Some(kern) = g.get_mut(kern_id) {
		kern.entities.insert(new_id.clone(), new_thought);
	}
	g.index_entity(&new_id, kern_id);

	let now = std::time::SystemTime::now();
	if let Some(kern) = g.get_mut(&old_kern_id) {
		if let Some(old) = kern.entities.get_mut(old_id) {
			old.status = EntityStatus::Superseded;
			old.superseded_by = new_id.clone();
			old.stamp_invalidated(now, new_valid_from);
		}
	}
	g.entity_idx.delete(old_id);
	g.gnn_entity_idx.delete(old_id);

	let vec = if !new_vec.is_empty() && !old_vec.is_empty() {
		average_vec(&new_vec, &old_vec)
	} else {
		Vec::new()
	};
	vec![commit_reason(
		g,
		kern_id,
		&new_id,
		old_id,
		ReasonKind::Supersedes,
		1.0,
		vec,
	)]
}

pub fn get_or_spawn_unnamed_child(g: &mut GraphGnn, kern_id: &str) -> String {
	// Use `get` (auto-loads), NOT `loaded`: an evicted child would otherwise be
	// respawned every call — the runaway that filled the graph with unnamed kerns.
	let children = g
		.get(kern_id)
		.map(|k| k.children.clone())
		.unwrap_or_default();
	for child_id in &children {
		if let Some(c) = g.get(child_id) {
			if c.is_unnamed() {
				return child_id.clone();
			}
		}
	}
	spawn_unnamed_child(g, kern_id)
}

// Always creates a FRESH unnamed child (one per call). For the single reusable
// holding-pen child use get_or_spawn_unnamed_child.
pub fn spawn_unnamed_child(g: &mut GraphGnn, kern_id: &str) -> String {
	let root_id = g
		.get(kern_id)
		.map(|k| k.root_id.clone())
		.unwrap_or_default();
	let child = Kern::new_unnamed(kern_id, &root_id);
	let child_id = child.id.clone();
	g.register(child);
	if let Some(kern) = g.get_mut(kern_id) {
		kern.children.push(child_id.clone());
	}
	child_id
}

// The generic catch-all: empty anchor_vec never matches routing; named, hence immortal.
pub(crate) fn get_or_spawn_generic_child(g: &mut GraphGnn, parent_id: &str) -> String {
	// Use `get` (auto-loads), NOT `loaded`: even the immortal generic child can
	// spill to disk — same duplicate-spawn runaway as get_or_spawn_unnamed_child.
	let children = g
		.get(parent_id)
		.map(|k| k.children.clone())
		.unwrap_or_default();
	for child_id in &children {
		if let Some(c) = g.get(child_id) {
			if c.anchor_text == GENERIC_ANCHOR {
				return child_id.clone();
			}
		}
	}
	let root_id = g
		.get(parent_id)
		.map(|k| k.root_id.clone())
		.unwrap_or_default();
	let child = Kern::new_named_child(parent_id, &root_id, GENERIC_ANCHOR, Vec::new());
	let child_id = child.id.clone();
	g.register(child);
	if let Some(kern) = g.get_mut(parent_id) {
		kern.children.push(child_id.clone());
	}
	child_id
}

// One anchor per name: a same-normalized-name anchor is updated in place.
pub(crate) fn add_anchor(g: &mut GraphGnn, name: &str, vec: Vec<f32>) {
	if let Some(existing) = find_anchor_by_name(g, name) {
		if let Some(k) = g.get_mut(&existing) {
			k.anchor_vec = vec;
		}
		return;
	}
	let root = g.root.id.clone();
	let root_net = g.root.root_id.clone();
	let child = Kern::new_named_child(&root, &root_net, name, vec);
	let cid = child.id.clone();
	g.register(child);
	if let Some(r) = g.get_mut(&root) {
		if !r.children.contains(&cid) {
			r.children.push(cid);
		}
	}
}

fn find_anchor_by_name(g: &GraphGnn, name: &str) -> Option<String> {
	let needle = name.trim().to_lowercase();
	root_anchor_ids(g).into_iter().find(|cid| {
		g.loaded(cid)
			.map(|c| c.anchor_text.trim().to_lowercase() == needle)
			.unwrap_or(false)
	})
}

fn equivalent_anchor_exists(g: &GraphGnn, name: &str, vec: &[f32]) -> bool {
	if find_anchor_by_name(g, name).is_some() {
		return true;
	}
	if vec.is_empty() {
		return false;
	}
	root_anchor_ids(g).into_iter().any(|cid| {
		g.loaded(&cid)
			.map(|c| {
				!c.anchor_vec.is_empty()
					&& crate::base::math::cosine(&c.anchor_vec, vec)
						>= crate::base::constants::ANCHOR_DEDUP_THRESHOLD
			})
			.unwrap_or(false)
	})
}

// Read from the kern map, not the g.root snapshot — runtime mutations land there.
pub(crate) fn root_anchor_ids(g: &GraphGnn) -> Vec<String> {
	let root = g.root.id.clone();
	let children = g
		.loaded(&root)
		.map(|r| r.children.clone())
		.unwrap_or_default();
	children
		.into_iter()
		.filter(|cid| {
			g.loaded(cid)
				.map(|c| !c.anchor_text.is_empty() && c.anchor_text != GENERIC_ANCHOR)
				.unwrap_or(false)
		})
		.collect()
}

pub(crate) fn promote_to_root_if_generic(g: &mut GraphGnn, kern_id: &str) -> bool {
	let parent_id = match g.loaded(kern_id) {
		Some(k) => k.parent.clone(),
		None => return false,
	};
	let under_generic = g
		.loaded(&parent_id)
		.map(|p| p.anchor_text == GENERIC_ANCHOR)
		.unwrap_or(false);
	if !under_generic {
		return false;
	}
	let (cand_name, cand_vec) = match g.loaded(kern_id) {
		Some(k) => (k.anchor_text.clone(), k.anchor_vec.clone()),
		None => return false,
	};
	if equivalent_anchor_exists(g, &cand_name, &cand_vec) {
		return false;
	}
	let root_id = g.root.id.clone();
	if let Some(gen_kern) = g.get_mut(&parent_id) {
		gen_kern.children.retain(|c| c.as_str() != kern_id);
	}
	if let Some(k) = g.get_mut(kern_id) {
		k.parent = root_id.clone();
	}
	if let Some(root) = g.get_mut(&root_id) {
		if !root.children.iter().any(|c| c.as_str() == kern_id) {
			root.children.push(kern_id.to_string());
		}
	}
	true
}

pub(crate) fn remove_anchor(g: &mut GraphGnn, name: &str) -> bool {
	let root = g.root.id.clone();
	let generic = get_or_spawn_generic_child(g, &root);
	let target = root_anchor_ids(g).into_iter().find(|cid| {
		*cid != generic
			&& g
				.loaded(cid)
				.map(|c| c.anchor_text == name)
				.unwrap_or(false)
	});
	let Some(tid) = target else {
		return false;
	};
	if let Some(t) = g.get_mut(&tid) {
		t.anchor_text.clear();
		t.anchor_vec.clear();
		t.parent = generic.clone();
	}
	if let Some(r) = g.get_mut(&root) {
		r.children.retain(|c| c != &tid);
	}
	if let Some(gk) = g.get_mut(&generic) {
		gk.children.push(tid);
	}
	true
}

fn route_to_child_id(children: &[String], g: &GraphGnn, vec: &[f32]) -> Option<String> {
	let mut best_id = None;
	let mut best_p = 0.0;
	for id in children {
		let c = match g.loaded(id) {
			Some(k) if k.is_named() && !k.anchor_vec.is_empty() => k,
			_ => continue,
		};
		let dist = cosine_distance(vec, &c.anchor_vec);
		let p = acceptance_probability(dist, c.inner_radius, c.outer_radius);
		if p > best_p {
			best_p = p;
			best_id = Some(id.clone());
		}
	}
	if best_p < ACCEPT_FLOOR {
		return None;
	}
	best_id
}

pub fn acceptance_probability(dist: f64, inner: f64, outer: f64) -> f64 {
	if dist <= inner {
		1.0
	} else if dist >= outer {
		0.0
	} else {
		let x = (dist - inner) / (outer - inner);
		1.0 / (1.0 + (8.0 * (x - 0.5)).exp())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;

	fn ent(id: &str, vector: Vec<f32>) -> Entity {
		Entity {
			id: id.into(),
			vector,
			statements: vec!["x".into()],
			..Default::default()
		}
	}

	#[test]
	fn unnamed_child_reused_when_evicted_by_load_cap() {
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		g.set_store(std::sync::Arc::new(
			crate::base::store::Store::open(&g.data_dir).unwrap(),
		));
		g.set_max_loaded_kerns(1);
		let root = g.root.id.clone();

		let first = get_or_spawn_unnamed_child(&mut g, &root);
		for _ in 0..20 {
			let id = get_or_spawn_unnamed_child(&mut g, &root);
			assert_eq!(id, first, "must reuse the evicted unnamed child");
		}
		assert_eq!(g.count(), 2, "no runaway kern creation under the cap");
	}

	#[test]
	fn generic_child_reused_when_evicted_by_load_cap() {
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		g.set_store(std::sync::Arc::new(
			crate::base::store::Store::open(&g.data_dir).unwrap(),
		));
		g.set_max_loaded_kerns(1);
		let root = g.root.id.clone();

		let first = get_or_spawn_generic_child(&mut g, &root);
		for _ in 0..20 {
			let id = get_or_spawn_generic_child(&mut g, &root);
			assert_eq!(id, first, "must reuse the evicted generic child");
		}
		assert_eq!(
			g.count(),
			2,
			"exactly one generic child created, no runaway"
		);
	}

	#[test]
	fn unnamed_child_not_duplicated_when_non_root_parent_evicts() {
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		g.set_store(std::sync::Arc::new(
			crate::base::store::Store::open(&g.data_dir).unwrap(),
		));
		g.set_max_loaded_kerns(1);
		let root = g.root.id.clone();
		let root_net = g.root.root_id.clone();

		let parent = {
			let p = Kern::new_named_child(&root, &root_net, "parent-anchor", vec![1.0, 0.0]);
			let pid = p.id.clone();
			g.register(p);
			if let Some(r) = g.get_mut(&root) {
				if !r.children.contains(&pid) {
					r.children.push(pid.clone());
				}
			}
			pid
		};

		let first = get_or_spawn_unnamed_child(&mut g, &parent);
		for _ in 0..20 {
			let id = get_or_spawn_unnamed_child(&mut g, &parent);
			assert_eq!(
				id, first,
				"reuse the unnamed child even when the non-root parent evicted"
			);
		}
		assert_eq!(
			g.count(),
			3,
			"no runaway: root + parent + one unnamed child"
		);
	}

	#[test]
	fn supersede_drops_the_old_entity_from_the_search_index() {
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		let old = Entity {
			id: "old".into(),
			external_id: "ext1".into(),
			vector: vec![1.0, 0.0],
			status: EntityStatus::Active,
			..Default::default()
		};
		g.entity_idx.insert("old".into(), vec![1.0, 0.0]);
		if let Some(k) = g.get_mut(&kid) {
			k.entities.insert("old".into(), old);
			k.source_index.insert("ext1".into(), "old".into());
		}
		g.index_entity("old", &kid);
		g.set_source_entry("ext1".into(), kid.clone());

		let before: Vec<String> = search_all_unlocked(&g, &[1.0, 0.0], 5)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert!(
			before.contains(&"old".to_string()),
			"old is indexed before supersede"
		);

		supersede(&mut g, &kid, "new", &[1.0, 0.0], "ext1");

		let after: Vec<String> = search_all_unlocked(&g, &[1.0, 0.0], 5)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert!(
			!after.contains(&"old".to_string()),
			"superseded entity removed from search index"
		);
		let kern = g.loaded(&kid).unwrap();
		let old_e = kern
			.entities
			.get("old")
			.expect("superseded entity still stored");
		assert_eq!(
			old_e.status,
			EntityStatus::Superseded,
			"kept as Superseded history"
		);
		assert_eq!(old_e.superseded_by, "new", "supersede chain preserved");
	}

	#[test]
	fn accept_never_leaves_empty_unnamed_kern() {
		let (mut g, root, _anchor) = graph_with_anchor();
		let vectors = [
			vec![1.0, 0.0, 0.0], // matches the anchor
			vec![1.0, 0.0, 0.0], // duplicate -> deduped, must NOT spawn
			vec![0.0, 1.0, 0.0], // non-match -> generic
			vec![0.0, 1.0, 0.0], // duplicate of the generic one
			vec![0.0, 0.0, 1.0], // another non-match
			vec![0.9, 0.1, 0.0], // near the anchor
		];
		for (i, v) in vectors.iter().enumerate() {
			accept(&mut g, &root, ent(&format!("e{i}"), v.clone()), "");
		}
		let empties: Vec<String> = g
			.all()
			.iter()
			.filter(|k| k.id != root && k.is_unnamed() && k.entities.is_empty())
			.map(|k| k.id.clone())
			.collect();
		assert!(
			empties.is_empty(),
			"accept left empty unnamed kern(s) behind: {empties:?}"
		);
	}

	#[test]
	fn supersede_stamps_both_temporal_clocks() {
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		let old = Entity {
			id: "old".into(),
			external_id: "ext1".into(),
			vector: vec![1.0, 0.0],
			status: EntityStatus::Active,
			created_at: Some(std::time::SystemTime::now()),
			..Default::default()
		};
		g.entity_idx.insert("old".into(), vec![1.0, 0.0]);
		let new_from = std::time::SystemTime::now();
		let new = Entity {
			id: "new".into(),
			external_id: "ext1".into(),
			vector: vec![1.0, 0.0],
			status: EntityStatus::Active,
			valid_from: Some(new_from),
			..Default::default()
		};
		if let Some(k) = g.get_mut(&kid) {
			k.entities.insert("old".into(), old);
			k.entities.insert("new".into(), new);
			k.source_index.insert("ext1".into(), "old".into());
		}
		g.index_entity("old", &kid);
		g.index_entity("new", &kid);
		g.set_source_entry("ext1".into(), kid.clone());

		supersede(&mut g, &kid, "new", &[1.0, 0.0], "ext1");

		let kern = g.loaded(&kid).unwrap();
		let old_e = kern.entities.get("old").unwrap();
		assert_eq!(old_e.status, EntityStatus::Superseded);
		assert!(
			old_e.invalidated_at.is_some(),
			"transaction-time stamp recorded"
		);
		assert_eq!(
			old_e.valid_to,
			Some(new_from),
			"old window closes at the successor's valid_from"
		);
		assert!(
			!old_e.is_valid_at(new_from),
			"old is no longer valid at the successor's start instant"
		);
	}

	#[test]
	fn contradiction_supersede_materializes_new_and_invalidates_old() {
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		let old = Entity {
			id: "old".into(),
			vector: vec![1.0, 0.0],
			status: EntityStatus::Active,
			created_at: Some(std::time::SystemTime::now()),
			..Default::default()
		};
		g.entity_idx.insert("old".into(), vec![1.0, 0.0]);
		if let Some(k) = g.get_mut(&kid) {
			k.entities.insert("old".into(), old);
		}
		g.index_entity("old", &kid);

		let new = Entity {
			id: "new".into(),
			vector: vec![0.99, 0.01],
			status: EntityStatus::Active,
			created_at: Some(std::time::SystemTime::now()),
			..Default::default()
		};
		let rids = supersede_by_contradiction(&mut g, &kid, "old", new);
		assert_eq!(rids.len(), 1, "one Supersedes edge minted");

		let kern = g.loaded(&kid).unwrap();
		assert!(
			kern.entities.contains_key("new"),
			"new revision materialized"
		);
		let old_e = kern.entities.get("old").unwrap();
		assert_eq!(old_e.status, EntityStatus::Superseded);
		assert_eq!(old_e.superseded_by, "new");
		assert!(old_e.invalidated_at.is_some(), "old stamped invalidated");

		let hits: Vec<String> = search_all_unlocked(&g, &[1.0, 0.0], 5)
			.into_iter()
			.map(|h| h.entity_id)
			.collect();
		assert!(!hits.contains(&"old".to_string()), "old evicted from ANN");
		assert!(hits.contains(&"new".to_string()), "new revision indexed");
	}

	#[test]
	fn contradiction_supersede_is_a_noop_on_missing_or_already_superseded() {
		let mut g = GraphGnn::new();
		let kid = g.root.id.clone();
		let new = Entity {
			id: "new".into(),
			vector: vec![1.0, 0.0],
			..Default::default()
		};
		assert!(supersede_by_contradiction(&mut g, &kid, "ghost", new).is_empty());
	}

	#[test]
	fn parse_contradiction_fails_open_to_related() {
		assert_eq!(parse_contradiction("UPDATE"), ContradictionClass::Supersede);
		assert_eq!(
			parse_contradiction("  contradiction \n"),
			ContradictionClass::Supersede
		);
		assert_eq!(parse_contradiction("RELATED"), ContradictionClass::Related);
		assert_eq!(parse_contradiction(""), ContradictionClass::Related);
		assert_eq!(
			parse_contradiction("I'm not sure"),
			ContradictionClass::Related
		);
		assert_eq!(
			parse_contradiction("this is an update but they are RELATED"),
			ContradictionClass::Related,
			"a RELATED mention wins — conservative"
		);
	}

	#[test]
	fn duplicate_vector_is_deduped() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let r1 = accept(&mut g, &root, ent("a", vec![1.0, 0.0, 0.0]), "");
		assert!(!r1.deduped, "first entity is placed, not deduped");
		let r2 = accept(&mut g, &root, ent("b", vec![1.0, 0.0, 0.0]), "");
		assert!(r2.deduped, "identical vector must dedup");
	}

	#[test]
	fn distinct_vector_is_placed() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		accept(&mut g, &root, ent("a", vec![1.0, 0.0, 0.0]), "");
		let r = accept(&mut g, &root, ent("c", vec![0.0, 1.0, 0.0]), "");
		assert!(!r.deduped, "orthogonal vector must not dedup");
	}

	fn graph_with_anchor() -> (GraphGnn, String, String) {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let root_net = g.root.root_id.clone();
		let anchor = Kern::new_named_child(&root, &root_net, "work", vec![1.0, 0.0, 0.0]);
		let anchor_id = anchor.id.clone();
		g.register(anchor);
		g.get_mut(&root).unwrap().children.push(anchor_id.clone());
		(g, root, anchor_id)
	}

	#[test]
	fn routes_nonmatch_to_generic() {
		let (mut g, root, anchor_id) = graph_with_anchor();
		let r = accept(&mut g, &root, ent("e", vec![0.0, 1.0, 0.0]), "");
		assert_ne!(
			r.placed_in, root,
			"must not commit onto the root dispatcher"
		);
		assert_ne!(
			r.placed_in, anchor_id,
			"non-matching entity must not enter the anchor"
		);
		let placed = g.loaded(&r.placed_in).expect("placed kern is loaded");
		assert_eq!(
			placed.anchor_text, GENERIC_ANCHOR,
			"fell through to generic"
		);
	}

	#[test]
	fn routes_match_to_anchor() {
		let (mut g, root, anchor_id) = graph_with_anchor();
		let r = accept(&mut g, &root, ent("e", vec![1.0, 0.0, 0.0]), "");
		assert_eq!(r.placed_in, anchor_id, "matching entity enters its anchor");
	}

	fn anchor_names(g: &GraphGnn) -> Vec<String> {
		root_anchor_ids(g)
			.iter()
			.filter_map(|c| g.loaded(c))
			.map(|k| k.anchor_text.clone())
			.collect()
	}

	#[test]
	fn add_anchor_creates_named_root_child() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		add_anchor(&mut g, "work", vec![1.0, 0.0, 0.0]);
		assert!(anchor_names(&g).contains(&"work".to_string()));
		let r = accept(&mut g, &root, ent("e", vec![1.0, 0.0, 0.0]), "");
		assert!(
			g.loaded(&r.placed_in)
				.map(|k| k.anchor_text == "work")
				.unwrap_or(false),
			"matching entity enters the added anchor"
		);
	}

	#[test]
	fn remove_anchor_demotes_and_reports() {
		let mut g = GraphGnn::new();
		add_anchor(&mut g, "work", vec![1.0, 0.0, 0.0]);
		assert!(remove_anchor(&mut g, "work"), "existing anchor removed");
		assert!(
			!anchor_names(&g).contains(&"work".to_string()),
			"anchor no longer a named root child"
		);
		assert!(!remove_anchor(&mut g, "missing"), "missing anchor -> false");
	}

	#[test]
	fn promote_skips_when_root_has_equivalent_anchor_by_name() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		add_anchor(&mut g, "sessions with no parent", vec![1.0, 0.0, 0.0]);
		let generic = get_or_spawn_generic_child(&mut g, &root);
		let root_net = g.root.root_id.clone();
		let child = Kern::new_named_child(
			&generic,
			&root_net,
			" Sessions With No Parent ",
			vec![0.0, 1.0, 0.0],
		);
		let cid = child.id.clone();
		g.register(child);
		g.get_mut(&generic).unwrap().children.push(cid.clone());

		assert!(
			!promote_to_root_if_generic(&mut g, &cid),
			"name-equivalent anchor exists -> no promotion"
		);
		assert!(
			!root_anchor_ids(&g).contains(&cid),
			"not minted as a root anchor"
		);
		assert_eq!(
			g.loaded(&cid).unwrap().parent,
			generic,
			"stays under generic"
		);
	}

	#[test]
	fn promote_skips_when_root_anchor_vec_is_near_duplicate() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		add_anchor(&mut g, "parentless sessions", vec![1.0, 0.0, 0.0]);
		let generic = get_or_spawn_generic_child(&mut g, &root);
		let root_net = g.root.root_id.clone();

		let near = Kern::new_named_child(
			&generic,
			&root_net,
			"sessions without parents",
			vec![1.0, 0.1, 0.0],
		);
		let near_id = near.id.clone();
		g.register(near);
		g.get_mut(&generic).unwrap().children.push(near_id.clone());
		assert!(
			!promote_to_root_if_generic(&mut g, &near_id),
			"vector-equivalent anchor exists -> no promotion"
		);

		let fresh = Kern::new_named_child(&generic, &root_net, "shader pipelines", vec![0.0, 0.0, 1.0]);
		let fresh_id = fresh.id.clone();
		g.register(fresh);
		g.get_mut(&generic).unwrap().children.push(fresh_id.clone());
		assert!(
			promote_to_root_if_generic(&mut g, &fresh_id),
			"orthogonal concept still promotes"
		);
	}

	#[test]
	fn add_anchor_updates_existing_same_name_instead_of_minting_duplicate() {
		let mut g = GraphGnn::new();
		add_anchor(&mut g, "work", vec![1.0, 0.0, 0.0]);
		add_anchor(&mut g, "work", vec![0.0, 1.0, 0.0]);

		let ids: Vec<String> = root_anchor_ids(&g)
			.into_iter()
			.filter(|cid| {
				g.loaded(cid)
					.map(|c| c.anchor_text == "work")
					.unwrap_or(false)
			})
			.collect();
		assert_eq!(ids.len(), 1, "one anchor per name, not one per call");
		let vec = g.loaded(&ids[0]).unwrap().anchor_vec.clone();
		assert_eq!(
			vec,
			vec![0.0, 1.0, 0.0],
			"second call updates the routing vector in place"
		);
	}

	#[test]
	fn promotes_generic_child_to_root() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let generic = get_or_spawn_generic_child(&mut g, &root);
		let root_net = g.root.root_id.clone();
		let child = Kern::new_named_child(&generic, &root_net, "shaders", vec![1.0, 0.0, 0.0]);
		let cid = child.id.clone();
		g.register(child);
		g.get_mut(&generic).unwrap().children.push(cid.clone());

		assert!(
			promote_to_root_if_generic(&mut g, &cid),
			"promoted out of generic"
		);
		assert!(
			root_anchor_ids(&g).contains(&cid),
			"now a root-level anchor"
		);
		assert_eq!(
			g.loaded(&cid).unwrap().parent,
			root,
			"parent rewired to root"
		);
		assert!(
			!g.loaded(&generic).unwrap().children.contains(&cid),
			"detached from generic"
		);
		assert!(
			!promote_to_root_if_generic(&mut g, &cid),
			"idempotent once at root level"
		);
	}
}
