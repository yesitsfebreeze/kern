use crate::base::graph::GraphGnn;
use crate::base::math::cosine;
use crate::base::search::EntityHit;
use crate::base::types::*;
use crate::config::RetrievalConfig;
use crate::retrieval::seed::Weights;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct PathChain {
	pub nodes: Vec<String>,
	pub score: f64,
}

#[derive(Debug, Clone)]
pub struct ScoredEntity {
	pub entity: Entity,
	pub score: f64,
}

/// A scored entity borrowed from the graph — the pipeline works on these, cloning
/// to owned [`ScoredEntity`] only for delivery survivors.
#[derive(Debug, Clone, Copy)]
pub struct ScoredRef<'a> {
	pub entity: &'a Entity,
	pub score: f64,
}

impl ScoredRef<'_> {
	pub fn to_owned(self) -> ScoredEntity {
		ScoredEntity {
			entity: self.entity.clone(),
			score: self.score,
		}
	}
}

/// Uniform view over owned [`ScoredEntity`] and borrowed [`ScoredRef`] so the
/// scoring/diversify stages run on either without cloning.
pub trait Scored {
	fn entity(&self) -> &Entity;
	fn score(&self) -> f64;
	fn set_score(&mut self, score: f64);
}

impl Scored for ScoredEntity {
	fn entity(&self) -> &Entity {
		&self.entity
	}
	fn score(&self) -> f64 {
		self.score
	}
	fn set_score(&mut self, score: f64) {
		self.score = score;
	}
}

impl Scored for ScoredRef<'_> {
	fn entity(&self) -> &Entity {
		self.entity
	}
	fn score(&self) -> f64 {
		self.score
	}
	fn set_score(&mut self, score: f64) {
		self.score = score;
	}
}

pub struct ExpandResult<'a> {
	pub scored: Vec<ScoredRef<'a>>,
	pub chains: Vec<PathChain>,
}

/// Assigns a dense `u32` to each distinct entity id in one [`expand`] run: the id
/// clones into an `Rc<str>` once (on intern), every later touch is a `u32` lookup.
#[derive(Default)]
struct Interner {
	idx: HashMap<Rc<str>, u32>,
	names: Vec<Rc<str>>,
}

impl Interner {
	fn intern(&mut self, s: &str) -> u32 {
		if let Some(&i) = self.idx.get(s) {
			return i;
		}
		let rc: Rc<str> = Rc::from(s);
		let i = self.names.len() as u32;
		self.names.push(Rc::clone(&rc));
		self.idx.insert(rc, i);
		i
	}

	fn name(&self, i: u32) -> &str {
		&self.names[i as usize]
	}

	/// Owned handle to id `i` (a refcount bump) — held as `Rc<str>` not `&str` so
	/// the loop can keep mutating the interner (interning neighbours) meanwhile.
	fn name_rc(&self, i: u32) -> Rc<str> {
		Rc::clone(&self.names[i as usize])
	}
}

/// One node of the beam's path forest. A seed root has no edge (`rid == ""`) and
/// no parent ([`NO_PARENT`]).
struct ChainNode<'g> {
	ent: u32,
	rid: &'g str,
	parent: u32,
}

const NO_PARENT: u32 = u32::MAX;

/// One frontier entry. The payload (interned id, arena index) never participates
/// in ordering; only `score` does.
struct BeamNode {
	ent: u32,
	score: f64,
	chain: u32,
}

/// Binary max-heap over [`BeamNode`]s keyed on `score` (assumed finite).
/// Hand-rolled so the u32/arena payload stays out of the ordering.
#[derive(Default)]
struct Beam {
	items: Vec<BeamNode>,
}

impl Beam {
	fn push(&mut self, node: BeamNode) {
		self.items.push(node);
		let mut i = self.items.len() - 1;
		while i > 0 {
			let p = (i - 1) / 2;
			if self.items[i].score <= self.items[p].score {
				break;
			}
			self.items.swap(i, p);
			i = p;
		}
	}

	fn pop(&mut self) -> Option<BeamNode> {
		if self.items.is_empty() {
			return None;
		}
		let n = self.items.len() - 1;
		self.items.swap(0, n);
		let top = self.items.pop().unwrap();
		let sz = self.items.len();
		let mut i = 0;
		loop {
			let (l, r) = (2 * i + 1, 2 * i + 2);
			let mut s = i;
			if l < sz && self.items[l].score > self.items[s].score {
				s = l;
			}
			if r < sz && self.items[r].score > self.items[s].score {
				s = r;
			}
			if s == i {
				break;
			}
			self.items.swap(i, s);
			i = s;
		}
		Some(top)
	}
}

/// Walk `node`'s parent chain into the `[seed, rid, ent, rid, ent, …]` id list
/// [`PathChain`] carries.
fn materialize_chain(arena: &[ChainNode], interner: &Interner, mut node: u32) -> Vec<String> {
	let mut nodes: Vec<String> = Vec::new();
	loop {
		let n = &arena[node as usize];
		nodes.push(interner.name(n.ent).to_string());
		if n.parent == NO_PARENT {
			break;
		}
		nodes.push(n.rid.to_string());
		node = n.parent;
	}
	nodes.reverse();
	nodes
}

pub fn expand<'a>(
	g: &'a GraphGnn,
	cfg: &RetrievalConfig,
	query_vec: &'a [f32],
	seeds: &[EntityHit],
	w: Weights,
) -> ExpandResult<'a> {
	let mut interner = Interner::default();
	let mut heap = Beam::default();
	let mut arena: Vec<ChainNode> = Vec::new();
	let mut visited: HashSet<u32> = HashSet::new();
	let mut results: HashMap<u32, f64> = HashMap::new();
	let mut chains: Vec<PathChain> = Vec::new();
	let mut global_best: f64 = 0.0;

	for s in seeds {
		let ent = interner.intern(&s.entity_id);
		let chain = arena.len() as u32;
		arena.push(ChainNode {
			ent,
			rid: "",
			parent: NO_PARENT,
		});
		heap.push(BeamNode {
			ent,
			score: s.score,
			chain,
		});
	}

	let max_expansions = cfg.max_expansions;
	let decay = cfg.decay;
	let refine_tw = cfg.refine_traversal_weight;
	let refine_cap = cfg.refine_boost_cap;
	let mut expansions = 0;

	while let Some(item) = heap.pop() {
		if expansions >= max_expansions {
			break;
		}
		expansions += 1;

		if !visited.insert(item.ent) {
			continue;
		}

		let entry = results.entry(item.ent).or_insert(0.0);
		if item.score > *entry {
			*entry = item.score;
		}

		if item.score > global_best {
			global_best = item.score;
		}
		let threshold = global_best * decay;

		if arena[item.chain as usize].parent != NO_PARENT {
			chains.push(PathChain {
				nodes: materialize_chain(&arena, &interner, item.chain),
				score: item.score,
			});
		}

		let item_name = interner.name_rc(item.ent);
		let name: &str = &item_name;
		let Some((_thought, kern)) = find_entity_and_kern(g, name) else {
			continue;
		};
		let edges = kern
			.by_from
			.get(name)
			.into_iter()
			.flatten()
			.chain(kern.by_to.get(name).into_iter().flatten());
		for rid in edges {
			let Some(reason) = kern.reasons.get(rid) else {
				continue;
			};
			if reason.is_remote() {
				continue;
			}
			if reason.kind == ReasonKind::Spawn && !reason.to.is_empty() {
				continue;
			}
			let neighbor_id = if reason.from == name {
				reason.to.as_str()
			} else {
				reason.from.as_str()
			};
			if neighbor_id.is_empty() {
				continue;
			}
			let nu = interner.intern(neighbor_id);
			if visited.contains(&nu) {
				continue;
			}
			let Some((neighbor, _)) = find_entity_and_kern(g, neighbor_id) else {
				continue;
			};
			let score = score_neighbor(query_vec, neighbor, reason, w, refine_tw, refine_cap);
			if score < threshold {
				continue;
			}
			let chain = arena.len() as u32;
			arena.push(ChainNode {
				ent: nu,
				rid: rid.as_str(),
				parent: item.chain,
			});
			heap.push(BeamNode {
				ent: nu,
				score,
				chain,
			});
		}
	}

	let scored: Vec<ScoredRef<'a>> = results
		.into_iter()
		.filter_map(|(id, score)| {
			find_entity_and_kern(g, interner.name(id)).map(|(t, _)| ScoredRef { entity: t, score })
		})
		.collect();

	ExpandResult { scored, chains }
}

pub fn score_neighbor(
	query_vec: &[f32],
	neighbor: &Entity,
	reason: &Reason,
	w: Weights,
	refine_traversal_weight: f64,
	refine_boost_cap: f64,
) -> f64 {
	let content_score = if neighbor.has_vector() {
		cosine(query_vec, &neighbor.vector)
	} else {
		0.0
	};
	let reason_score = if reason.has_vector() {
		cosine(query_vec, &reason.vector)
	} else {
		0.0
	};
	let traversal_boost = ((reason.traversal_count.value() as f64 + 1.0).ln()
		* refine_traversal_weight)
		.min(refine_boost_cap);
	let edge_score = (reason.score.clamp(0.0, 1.0) + traversal_boost).min(1.0);

	w.content * content_score + w.reason * reason_score + w.edge * edge_score
}

/// Resolve an entity and its owning kern by reference. Two-pass: O(1) via the
/// `kern_of_entity` index, then a full scan as fallback for stale/missing index entries.
fn find_entity_and_kern<'a>(g: &'a GraphGnn, id: &str) -> Option<(&'a Entity, &'a Kern)> {
	if let Some(kid) = g.kern_of_entity(id) {
		if let Some(kern) = g.loaded(kid) {
			if let Some(t) = kern.entities.get(id) {
				return Some((t, kern));
			}
		}
	}
	for kern in g.all() {
		if let Some(t) = kern.entities.get(id) {
			return Some((t, kern));
		}
	}
	None
}

pub fn find_entity_ref_in_graph<'a>(g: &'a GraphGnn, id: &str) -> Option<&'a Entity> {
	find_entity_and_kern(g, id).map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::reason::add_reason;

	use crate::test_support::entity_vec as ent;
	fn edge(from: &str, to: &str, score: f64) -> Reason {
		Reason {
			id: format!("{from}->{to}"),
			from: from.into(),
			to: to.into(),
			score,
			kind: ReasonKind::Similarity,
			..Default::default()
		}
	}

	#[test]
	fn score_neighbor_pure_content_weight_is_cosine() {
		let neighbor = ent("n", vec![1.0, 0.0]);
		let r = edge("a", "n", 0.5); // no reason vector -> reason component 0
		let w = Weights {
			content: 1.0,
			reason: 0.0,
			edge: 0.0,
			lexical: 0.0,
		};
		let s = score_neighbor(&[1.0, 0.0], &neighbor, &r, w, 0.1, 0.3);
		assert!(
			(s - 1.0).abs() < 1e-9,
			"query aligned with neighbour -> 1.0"
		);
	}

	#[test]
	fn score_neighbor_pure_edge_weight_uses_clamped_reason_score() {
		let neighbor = ent("n", vec![]); // no vector -> content 0
		let r = edge("a", "n", 0.4); // traversal_count 0 -> ln(1)*tw = 0 boost
		let w = Weights {
			content: 0.0,
			reason: 0.0,
			edge: 1.0,
			lexical: 0.0,
		};
		let s = score_neighbor(&[1.0, 0.0], &neighbor, &r, w, 0.1, 0.3);
		assert!(
			(s - 0.4).abs() < 1e-9,
			"edge component is the clamped reason score"
		);
	}

	#[test]
	fn expand_walks_edges_from_seed_and_records_a_chain() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for id in ["a", "b", "c"] {
			k.entities.insert(id.into(), ent(id, vec![1.0, 0.0]));
		}
		add_reason(&mut k, edge("a", "b", 0.9));
		add_reason(&mut k, edge("b", "c", 0.9));
		g.kerns.insert("kx".into(), k);

		let cfg = RetrievalConfig::default();
		let seeds = [EntityHit {
			entity_id: "a".into(),
			score: 1.0,
		}];
		let w = Weights {
			content: 1.0,
			reason: 0.0,
			edge: 0.0,
			lexical: 0.0,
		};
		let res = expand(&g, &cfg, &[1.0, 0.0], &seeds, w);

		let ids: HashSet<&str> = res.scored.iter().map(|s| s.entity.id.as_str()).collect();
		assert!(ids.contains("a"), "the seed is scored");
		assert!(
			ids.contains("b"),
			"the 1-hop neighbour is reached via the edge"
		);
		assert!(
			res.chains.iter().any(|c| c.nodes.len() >= 3),
			"a multi-hop chain (entity, reason, entity) is recorded"
		);
	}
}
