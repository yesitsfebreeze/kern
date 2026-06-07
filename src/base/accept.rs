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
	// The dedup search is invariant in `thought.vector`: it scans entities
	// graph-wide (independent of the routing cursor) and routing only reads or
	// spawns empty child kerns, so the result cannot change during descent.
	// Compute it once here instead of re-running it on every loop iteration and
	// again in `commit_entity` (previously up to 65 identical HNSW searches).
	let is_dup = is_duplicate(g, &thought.vector);
	let target_id = route_entity(g, kern_id, &thought, is_dup);
	commit_entity(g, &target_id, thought, doc_id, is_dup)
}

/// Whether `vector` is within the dedup threshold of an existing entity.
fn is_duplicate(g: &GraphGnn, vector: &[f64]) -> bool {
	let hits = search_all_unlocked(g, vector, 1);
	!hits.is_empty() && hits[0].score > DEFAULT_DEDUP_THRESHOLD
}

fn route_entity(g: &mut GraphGnn, kern_id: &str, thought: &Entity, is_dup: bool) -> String {
	let mut current_id = kern_id.to_string();

	// A duplicate is committed in the starting kern; no descent needed.
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

		// The root is a pure dispatcher: it never gates on its own (possibly
		// stale) anchor. An entity that matched no named anchor at the root
		// falls through to the `generic` catch-all rather than committing onto
		// the root itself.
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
		g.entity_idx
			.insert(entity_id.clone(), thought_vec.clone());
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

fn add_similarity_reason(
	g: &mut GraphGnn,
	kern_id: &str,
	entity_id: &str,
	thought_vec: &[f64],
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

		let rid = reason_id(entity_id, &h.entity_id, ReasonKind::Similarity, "", "");
		let reason = Reason {
			id: rid.clone(),
			from: entity_id.to_string(),
			to: h.entity_id.clone(),
			to_kern_id: String::new(),
			to_net_id: String::new(),
			kind: ReasonKind::Similarity,
			dirty: false,
			text: String::new(),
			vector: vec.clone(),
			score: h.score,
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
		return vec![rid];
	}
	Vec::new()
}

fn add_provenance_reason(
	g: &mut GraphGnn,
	kern_id: &str,
	entity_id: &str,
	thought_vec: &[f64],
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

	let rid = reason_id(entity_id, doc_id, ReasonKind::Provenance, "", "");
	let reason = Reason {
		id: rid.clone(),
		from: entity_id.to_string(),
		to: doc_id.to_string(),
		to_kern_id: String::new(),
		to_net_id: String::new(),
		kind: ReasonKind::Provenance,
		dirty: false,
		text: String::new(),
		vector: vec.clone(),
		score: PROVENANCE_SCORE,
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
	vec![rid]
}

fn supersede(
	g: &mut GraphGnn,
	placed_kern_id: &str,
	entity_id: &str,
	thought_vec: &[f64],
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
			for kern in g.all() {
				if let Some(t) = kern.entities.get(&old_id) {
					found = Some((t.vector.clone(), kern.id.clone()));
					break;
				}
			}
		}
		match found {
			Some(f) => f,
			None => return Vec::new(),
		}
	};

	if let Some(kern) = g.get_mut(&old_kern_id) {
		if let Some(old) = kern.entities.get_mut(&old_id) {
			old.status = EntityStatus::Superseded;
			old.superseded_by = entity_id.to_string();
		}
	}

	let vec = if !thought_vec.is_empty() && !old_vec.is_empty() {
		average_vec(thought_vec, &old_vec)
	} else {
		Vec::new()
	};

	let rid = reason_id(entity_id, &old_id, ReasonKind::Supersedes, "", "");
	let reason = Reason {
		id: rid.clone(),
		from: entity_id.to_string(),
		to: old_id.clone(),
		to_kern_id: String::new(),
		to_net_id: String::new(),
		kind: ReasonKind::Supersedes,
		dirty: false,
		text: String::new(),
		vector: vec.clone(),
		score: 1.0,
		traversal_count: GCounter::new(),
		producer_id: String::new(),
	};

	if !vec.is_empty() {
		g.reason_idx.insert(rid.clone(), vec);
	}
	if let Some(kern) = g.get_mut(placed_kern_id) {
		add_reason(kern, reason);
	}
	g.index_reason(&rid, placed_kern_id);

	vec![rid]
}

pub fn get_or_spawn_unnamed_child(g: &mut GraphGnn, kern_id: &str) -> String {
	// Use `get` (auto-loads from disk), NOT `loaded` (in-memory only): under the
	// kern-load cap an existing unnamed child may be evicted to disk. Checking
	// only loaded kerns made this spawn a fresh unnamed child every call once the
	// cap started evicting — a runaway that filled the graph to `max_kerns`
	// unnamed kerns. Auto-loading finds and reuses the existing child instead.
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

/// Find the parent's permanent `generic` catch-all child, creating it if
/// absent. Generic carries an empty `anchor_vec` so similarity routing never
/// matches it; it is named, hence immortal (never GC'd).
pub(crate) fn get_or_spawn_generic_child(g: &mut GraphGnn, parent_id: &str) -> String {
	let children = g
		.loaded(parent_id)
		.map(|k| k.children.clone())
		.unwrap_or_default();
	for child_id in &children {
		if let Some(c) = g.loaded(child_id) {
			if c.anchor_text == GENERIC_ANCHOR {
				return child_id.clone();
			}
		}
	}
	let root_id = g
		.loaded(parent_id)
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

/// Create a named child of the root carrying `vec` as its routing vector — i.e.
/// a new anchor. Shared by the CLI `anchor add` and the MCP `anchor` tool.
pub(crate) fn add_anchor(g: &mut GraphGnn, name: &str, vec: Vec<f64>) {
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

/// The root's user-facing anchors — its named children excluding the `generic`
/// catch-all — read from the authoritative kern map (runtime mutations land
/// there, not on the `g.root` snapshot field).
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

/// Promote a kern to a first-class anchor under the root if it currently sits
/// directly under the `generic` catch-all. Called after the tick names a dense
/// generic cluster: the freshly-named kern graduates from generic to root level
/// so future matching memories route straight to it. Returns whether it moved.
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

/// Demote a named root anchor and reparent its kern under `generic`, so its
/// existing memories fall back to the catch-all. Returns whether an anchor of
/// that name was found and removed.
pub(crate) fn remove_anchor(g: &mut GraphGnn, name: &str) -> bool {
	let root = g.root.id.clone();
	let generic = get_or_spawn_generic_child(g, &root);
	let target = root_anchor_ids(g)
		.into_iter()
		.find(|cid| {
			*cid != generic && g.loaded(cid).map(|c| c.anchor_text == name).unwrap_or(false)
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

fn route_to_child_id(children: &[String], g: &GraphGnn, vec: &[f64]) -> Option<String> {
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
	// Floor: an entity only enters a named anchor if it clears ACCEPT_FLOOR.
	// Otherwise the caller routes it to the generic catch-all.
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

	fn ent(id: &str, vector: Vec<f64>) -> Entity {
		Entity {
			id: id.into(),
			vector,
			statements: vec!["x".into()],
			..Default::default()
		}
	}

	#[test]
	fn unnamed_child_reused_when_evicted_by_load_cap() {
		// Regression: under the kern-load cap, the unnamed child is evicted to
		// disk; `get_or_spawn_unnamed_child` must reload and REUSE it, not spawn
		// a fresh one each call (which previously ran the graph away to the cap).
		let dir = tempfile::tempdir().unwrap();
		let mut g = GraphGnn::new();
		g.data_dir = dir.path().to_string_lossy().into_owned();
		g.set_max_loaded_kerns(1); // only the root stays resident → child evicts
		let root = g.root.id.clone();

		let first = get_or_spawn_unnamed_child(&mut g, &root);
		for _ in 0..20 {
			let id = get_or_spawn_unnamed_child(&mut g, &root);
			assert_eq!(id, first, "must reuse the evicted unnamed child");
		}
		// Exactly one unnamed child ever created (root + 1), no runaway.
		assert_eq!(g.count(), 2, "no runaway kern creation under the cap");
	}

	#[test]
	fn duplicate_vector_is_deduped() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let r1 = accept(&mut g, &root, ent("a", vec![1.0, 0.0, 0.0]), "");
		assert!(!r1.deduped, "first entity is placed, not deduped");
		// Identical vector -> cosine 1.0 > dedup threshold -> deduped.
		let r2 = accept(&mut g, &root, ent("b", vec![1.0, 0.0, 0.0]), "");
		assert!(r2.deduped, "identical vector must dedup");
	}

	#[test]
	fn distinct_vector_is_placed() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		accept(&mut g, &root, ent("a", vec![1.0, 0.0, 0.0]), "");
		// Orthogonal vector -> cosine 0.0 < threshold -> placed, not deduped.
		let r = accept(&mut g, &root, ent("c", vec![0.0, 1.0, 0.0]), "");
		assert!(!r.deduped, "orthogonal vector must not dedup");
	}

	/// Build a root with one named anchor pointing at +x.
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
		// Orthogonal to the anchor -> p = 0 < ACCEPT_FLOOR -> falls through.
		let r = accept(&mut g, &root, ent("e", vec![0.0, 1.0, 0.0]), "");
		assert_ne!(r.placed_in, root, "must not commit onto the root dispatcher");
		assert_ne!(r.placed_in, anchor_id, "non-matching entity must not enter the anchor");
		let placed = g.loaded(&r.placed_in).expect("placed kern is loaded");
		assert_eq!(placed.anchor_text, GENERIC_ANCHOR, "fell through to generic");
	}

	#[test]
	fn routes_match_to_anchor() {
		let (mut g, root, anchor_id) = graph_with_anchor();
		// Aligned with the anchor -> dist 0 -> p = 1 >= ACCEPT_FLOOR.
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
		// A matching entity routes into the new anchor.
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
	fn promotes_generic_child_to_root() {
		let mut g = GraphGnn::new();
		let root = g.root.id.clone();
		let generic = get_or_spawn_generic_child(&mut g, &root);
		let root_net = g.root.root_id.clone();
		// A freshly-named kern sitting under generic (as the tick would leave it).
		let child = Kern::new_named_child(&generic, &root_net, "shaders", vec![1.0, 0.0, 0.0]);
		let cid = child.id.clone();
		g.register(child);
		g.get_mut(&generic).unwrap().children.push(cid.clone());

		assert!(promote_to_root_if_generic(&mut g, &cid), "promoted out of generic");
		assert!(root_anchor_ids(&g).contains(&cid), "now a root-level anchor");
		assert_eq!(g.loaded(&cid).unwrap().parent, root, "parent rewired to root");
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
