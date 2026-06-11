use crate::base::constants::{DEGRADE_DECAY_BASE, DEGRADE_DECAY_POW, DEGRADE_MIN_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::math::{average_vec, cosine, reason_id};
use crate::base::reason::{add_reason, remove_reason, remove_entity};
use crate::base::search::find_entity;
use crate::base::types::{Entity, Kern, Reason, ReasonKind};
use crate::base::util::{explain_relationship_prompt, short_id, truncate};

use super::{Client, Endpoint, load_graph, save_graph, with_graph};

/// Resolve a thought by exact id, then fall back to a unique id-prefix scan over
/// every kern. Display/lookup helper for `cmd_get` — lives here, not in the
/// dispatch/server module, since it is purely a graph read concern.
fn find_entity_by_prefix(g: &GraphGnn, id: &str) -> Option<(Entity, String)> {
	if let Some(pair) = find_entity(g, id) {
		return Some(pair);
	}
	for k in g.all() {
		for t in k.entities.values() {
			if t.id.starts_with(id) {
				return Some((t.clone(), k.id.clone()));
			}
		}
	}
	None
}

/// Pretty-print a kern and its children recursively (indented tree of thoughts
/// and edge counts). Display helper for `cmd_list`.
fn print_kern(kern: &Kern, g: &GraphGnn, depth: usize) {
	let indent = "  ".repeat(depth);
	let label = if kern.anchor_text.is_empty() {
		"[unnamed]".to_string()
	} else {
		kern.anchor_text.clone()
	};
	println!(
		"{}kern:{}  thoughts:{}  reasons:{}",
		indent,
		label,
		kern.entities.len(),
		kern.reasons.len(),
	);
	for t in kern.entities.values() {
		println!("{}  [{}] {}", indent, short_id(&t.id), truncate(&t.text(), 72));
	}
	for child_id in &kern.children {
		if let Some(child) = g.kerns.get(child_id) {
			print_kern(child, g, depth + 1);
		}
	}
}

pub(super) fn cmd_get(cfg: &crate::config::Config, id: &str) {
	let g = load_graph(cfg);
	let (thought, kern_id) = match find_entity_by_prefix(&g, id) {
		Some(pair) => pair,
		None => {
			// Lazy rehydrate: a thought evicted by stigmergy GC was spilled to
			// the cold tier (in the store) before being dropped from the hot graph.
			if let Some(e) = g.store().and_then(|s| s.cold_get(id).ok().flatten()) {
				println!("ID:     {}", e.id);
				println!("Kind:   {:?}", e.kind);
				println!("Score:  {:.4}", e.score);
				println!("Access: {}", e.access_count.value_i32());
				println!("Kern:   (cold)");
				println!("Text:   {}", e.text());
				return;
			}
			eprintln!("thought not found: {id}");
			return;
		}
	};

	println!("ID:     {}", thought.id);
	println!("Kind:   {:?}", thought.kind);
	println!("Score:  {:.4}", thought.score);
	println!("Access: {}", thought.access_count.value_i32());
	println!("Kern:   {}", short_id(&kern_id));
	println!("Text:   {}", thought.text());

	if let Some(kern) = g.kerns.get(&kern_id) {
		let rids = crate::base::reason::collect_reason_ids(kern, &thought.id);
		if !rids.is_empty() {
			println!("Edges:");
			for rid in &rids {
				if let Some(re) = kern.reasons.get(rid) {
					let dir = if re.from == thought.id { "->" } else { "<-" };
					let other = if re.from == thought.id {
						&re.to
					} else {
						&re.from
					};
					println!(
						"  {} {:?} score={:.4} {}  {}",
						dir,
						re.kind,
						re.score,
						short_id(other),
						truncate(&re.text, 80),
					);
				}
			}
		}
	}
}

pub(super) fn cmd_list(cfg: &crate::config::Config) {
	let g: GraphGnn = load_graph(cfg);
	print_kern(&g.root, &g, 0);
}

pub(super) fn cmd_forget(cfg: &crate::config::Config, id: &str) {
	with_graph(cfg, |g| match forget_entity(g, id) {
		Ok(removed) => println!("forgot {}  removed {} edges", short_id(id), removed),
		Err(e) => eprintln!("{e}: {id}"),
	});
}

/// Remove a non-fact thought and report how many incident edges went with it.
/// Facts are immutable and refused. Returns the edge delta on success, or a
/// static reason on rejection (unknown id / fact).
///
/// Pure graph mutation — no IO — so the fact-guard and edge bookkeeping are
/// unit-testable apart from `cmd_forget`'s graph load/save wrapper.
fn forget_entity(g: &mut GraphGnn, id: &str) -> Result<usize, &'static str> {
	let (thought, kern_id) = find_entity(g, id).ok_or("thought not found")?;
	if thought.is_fact() {
		return Err("cannot forget a fact");
	}
	let edges_before = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
	remove_entity(g, &kern_id, id);
	let edges_after = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
	// saturating: remove_entity only ever drops incident edges, never adds, but
	// guard the subtraction so a future change can't panic on underflow.
	Ok(edges_before.saturating_sub(edges_after))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn cmd_link(
	cfg: &crate::config::Config,
	from: &str,
	to: &str,
	reason: &str,
	embed_url: &str,
	embed_model: &str,
	reason_url: &str,
	reason_model: &str,
) {
	let mut g = load_graph(cfg);
	let (from_t, from_kern_id) = match find_entity(&g, from) {
		Some(pair) => pair,
		None => {
			eprintln!("from thought not found: {from}");
			return;
		}
	};
	let (to_t, _) = match find_entity(&g, to) {
		Some(pair) => pair,
		None => {
			eprintln!("to thought not found: {to}");
			return;
		}
	};

	let llm_client = Client::new(
		Endpoint::new(reason_url, reason_model, cfg.reason_key()),
		Endpoint::default(),
		Endpoint::new(embed_url, embed_model, &cfg.embed.key),
	);
	let mut reason_text = reason.to_string();

	if reason_text.is_empty() && !reason_url.is_empty() {
		let prompt = explain_relationship_prompt(&from_t.text(), &to_t.text());
		reason_text = llm_client
			.complete(&prompt)
			.await
			.unwrap_or_default()
			.trim()
			.to_string();
	}

	let reason_embed = if !reason_text.is_empty() {
		llm_client.embed(&reason_text).await.ok()
	} else {
		None
	};
	let vec = link_vector(reason_embed, &from_t.vector, &to_t.vector);

	let score = cosine(&from_t.vector, &to_t.vector);
	let rid = reason_id(from, to, ReasonKind::Similarity, &reason_text, "");
	let r = Reason {
		id: rid.clone(),
		from: from.to_string(),
		to: to.to_string(),
		kind: ReasonKind::Similarity,
		text: reason_text,
		vector: vec,
		score,
		..Default::default()
	};

	// Single mutable borrow of the owning kern. If it has vanished (shouldn't
	// happen — from_kern_id came from find_entity above), fail loudly instead of
	// the previous silent path that saved an unchanged graph yet still printed
	// "linked", reporting a success that never happened.
	let Some(kern) = g.kerns.get_mut(&from_kern_id) else {
		eprintln!("link failed: kern {} no longer present", short_id(&from_kern_id));
		return;
	};
	add_reason(kern, r);
	save_graph(&g);

	println!(
		"linked {} -> {}  edge={}  score={:.4}",
		short_id(from),
		short_id(to),
		short_id(&rid),
		score,
	);
}

/// Choose the vector stored on a new link edge: the embedded reason text when an
/// embedding was produced, otherwise the midpoint of the two endpoint vectors.
/// Pure — no IO — so the fallback policy is unit-testable apart from `cmd_link`.
fn link_vector(reason_embed: Option<Vec<f64>>, from_vec: &[f64], to_vec: &[f64]) -> Vec<f64> {
	reason_embed.unwrap_or_else(|| average_vec(from_vec, to_vec))
}

pub(super) fn cmd_degrade(cfg: &crate::config::Config, id: &str) {
	with_graph(cfg, |g| {
		let (_, kern_id) = match find_entity(g, id) {
			Some(pair) => pair,
			None => {
				eprintln!("thought not found: {id}");
				return;
			}
		};
		let (decayed, removed) = degrade_entity_reasons(g, &kern_id, id);
		println!(
			"degraded {}  decayed {} edges, removed {} below threshold",
			short_id(id),
			decayed,
			removed,
		);
	});
}

/// Down-weight every reason edge incident to `id` in kern `kern_id`: each edge's
/// score is cut by a geometric schedule (`BASE * POW^i`, so the i-th edge is
/// penalised less than the first), and any edge that would fall below
/// `DEGRADE_MIN_THRESHOLD` is removed outright. Returns `(decayed, removed)`.
///
/// Pure graph mutation — no IO — so the decay/removal policy is unit-testable in
/// isolation from `cmd_degrade`'s graph load/save wrapper.
fn degrade_entity_reasons(g: &mut GraphGnn, kern_id: &str, id: &str) -> (usize, usize) {
	let rids: Vec<String> = match g.kerns.get(kern_id) {
		Some(kern) => crate::base::reason::collect_reason_ids(kern, id),
		None => Vec::new(),
	};

	let mut decayed = 0usize;
	let mut removed = 0usize;
	for (i, rid) in rids.iter().enumerate() {
		let decay = DEGRADE_DECAY_BASE * DEGRADE_DECAY_POW.powi(i as i32);

		let should_remove = g
			.kerns
			.get(kern_id)
			.and_then(|kern| kern.reasons.get(rid))
			.map(|r| r.score - decay < DEGRADE_MIN_THRESHOLD)
			.unwrap_or(false);

		if should_remove {
			if let Some(kern) = g.kerns.get_mut(kern_id) {
				remove_reason(kern, rid);
			}
			removed += 1;
		} else if let Some(kern) = g.kerns.get_mut(kern_id) {
			if let Some(r) = kern.reasons.get_mut(rid) {
				r.score -= decay;
			}
		}
		decayed += 1;
	}
	(decayed, removed)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Kern, Reason};

	fn edge(from: &str, to: &str, score: f64) -> Reason {
		Reason {
			from: from.into(),
			to: to.into(),
			id: format!("{from}->{to}"),
			score,
			..Default::default()
		}
	}

	#[test]
	fn degrade_decays_survivors_and_removes_below_threshold() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		// a->b is healthy (score 1.0); a->c is already weak (score 0.0).
		// With BASE=0.15 the first-applied decay alone pushes 0.0 below the 0.05
		// floor, so a->c is removed while a->b survives, merely decayed.
		add_reason(&mut k, edge("a", "b", 1.0));
		add_reason(&mut k, edge("a", "c", 0.0));
		g.kerns.insert("kx".into(), k);

		let (decayed, removed) = degrade_entity_reasons(&mut g, "kx", "a");

		assert_eq!(decayed, 2, "both incident edges visited");
		assert_eq!(removed, 1, "the sub-threshold edge is reaped");

		let kern = g.kerns.get("kx").expect("kern present");
		assert_eq!(kern.reasons.len(), 1, "only the healthy edge remains");
		let survivor = kern.reasons.get("a->b").expect("a->b survives");
		assert!(survivor.score < 1.0, "survivor was decayed, not left untouched");
		assert!(survivor.score >= DEGRADE_MIN_THRESHOLD, "survivor stays above the floor");
	}

	#[test]
	fn degrade_on_unknown_kern_is_a_noop() {
		let mut g = GraphGnn::new();
		let (decayed, removed) = degrade_entity_reasons(&mut g, "missing", "a");
		assert_eq!((decayed, removed), (0, 0));
	}

	#[test]
	fn link_vector_prefers_the_reason_embedding() {
		let v = link_vector(Some(vec![1.0, 2.0, 3.0]), &[0.0, 0.0, 0.0], &[9.0, 9.0, 9.0]);
		assert_eq!(v, vec![1.0, 2.0, 3.0], "an embedded reason wins over the midpoint");
	}

	#[test]
	fn link_vector_falls_back_to_endpoint_midpoint() {
		let v = link_vector(None, &[0.0, 2.0], &[4.0, 6.0]);
		assert_eq!(v, vec![2.0, 4.0], "no embedding -> midpoint of the two endpoints");
	}

	use crate::base::types::EntityKind;

	fn ent(id: &str, kind: EntityKind) -> Entity {
		Entity { id: id.into(), kind, ..Default::default() }
	}

	fn graph_with(entities: &[(&str, EntityKind)], edges: &[(&str, &str)]) -> GraphGnn {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for (id, kind) in entities {
			k.entities.insert((*id).into(), ent(id, *kind));
		}
		for (from, to) in edges {
			add_reason(&mut k, edge(from, to, 1.0));
		}
		g.register(k); // populates entity_kern so find_entity hits the fast path
		g
	}

	#[test]
	fn forget_removes_thought_and_reports_edge_delta() {
		// a is linked to b and c; forgetting a must drop a and its two incident edges.
		let mut g = graph_with(
			&[("a", EntityKind::Claim), ("b", EntityKind::Claim), ("c", EntityKind::Claim)],
			&[("a", "b"), ("a", "c")],
		);
		let removed = forget_entity(&mut g, "a").expect("non-fact forget succeeds");
		assert_eq!(removed, 2, "both incident edges went with a");
		let kern = g.kerns.get("kx").expect("kern present");
		assert!(kern.entities.get("a").is_none(), "a is gone from the kern");
		assert!(kern.entities.contains_key("b"), "neighbours survive");
	}

	#[test]
	fn forget_refuses_a_fact() {
		let mut g = graph_with(&[("f", EntityKind::Fact)], &[]);
		assert_eq!(forget_entity(&mut g, "f"), Err("cannot forget a fact"));
		assert!(
			g.kerns.get("kx").unwrap().entities.contains_key("f"),
			"the fact is left intact"
		);
	}

	#[test]
	fn forget_unknown_id_is_rejected_not_panicked() {
		let mut g = graph_with(&[("a", EntityKind::Claim)], &[]);
		assert_eq!(forget_entity(&mut g, "nope"), Err("thought not found"));
	}

	#[test]
	fn find_entity_by_prefix_resolves_a_unique_prefix() {
		// cmd_get's fallback: no exact id match, but a unique id-prefix does.
		let g = graph_with(&[("abc123def", EntityKind::Claim)], &[]);
		let (hit, kern_id) = find_entity_by_prefix(&g, "abc12").expect("prefix resolves");
		assert_eq!(hit.id, "abc123def");
		assert_eq!(kern_id, "kx");
		// An exact id still resolves (the fast path before the prefix scan).
		assert!(find_entity_by_prefix(&g, "abc123def").is_some());
		// A prefix that matches nothing yields None, not a panic.
		assert!(find_entity_by_prefix(&g, "zzz").is_none());
	}
}
