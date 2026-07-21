use crate::base::constants::{DEGRADE_DECAY_BASE, DEGRADE_DECAY_POW, DEGRADE_MIN_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::math::{average_vec, reason_id};
use crate::base::reason::{add_reason, remove_entity, remove_reason};
use crate::base::search::find_entity;
use crate::base::types::{EntityKind, Kern, Reason, ReasonKind};
use crate::base::util::{explain_relationship_prompt, short_id, truncate};
use crate::mcp::tools_query::entity_detail_by_id;

use super::route::{array_field, f64_field, route, str_field, u64_field, Routed};
use super::{load_graph, with_graph, Client, Endpoint};

fn print_kern(kern: &Kern, g: &GraphGnn, depth: usize) {
	let indent = "  ".repeat(depth);
	let label = if kern.graviton_text.is_empty() {
		"[unnamed]".to_string()
	} else {
		kern.graviton_text.clone()
	};
	println!(
		"{}kern:{}  thoughts:{}  reasons:{}",
		indent,
		label,
		kern.entities.len(),
		kern.reasons.len(),
	);
	for t in kern.entities.values() {
		println!(
			"{}  [{}] {}",
			indent,
			short_id(&t.id),
			truncate(&t.text(), 72)
		);
	}
	for child_id in &kern.children {
		if let Some(child) = g.kerns.get(child_id) {
			print_kern(child, g, depth + 1);
		}
	}
}

// The detail JSON carries kinds as discriminants; the label is what the CLI has
// always printed, and an unmapped number is shown rather than guessed at.
fn entity_kind_label(n: u64) -> String {
	match u8::try_from(n).ok().and_then(EntityKind::from_u8) {
		Some(k) => format!("{k:?}"),
		None => n.to_string(),
	}
}

fn reason_kind_label(n: u64) -> String {
	match i32::try_from(n).ok().and_then(ReasonKind::from_i32) {
		Some(k) => format!("{k:?}"),
		None => n.to_string(),
	}
}

fn print_detail(v: &serde_json::Value) {
	let id = str_field(v, "id");
	println!("ID:     {id}");
	println!("Kind:   {}", entity_kind_label(u64_field(v, "kind")));
	println!("Score:  {:.4}", f64_field(v, "score"));
	println!("Access: {}", u64_field(v, "access_count"));
	println!("Kern:   {}", short_id(str_field(v, "kern")));
	println!("Text:   {}", str_field(v, "text"));

	let edges = array_field(v, "edges");
	if edges.is_empty() {
		return;
	}
	println!("Edges:");
	for e in edges {
		let from = str_field(e, "from");
		let outgoing = from == id;
		println!(
			"  {} {} score={:.4} {}  {}",
			if outgoing { "->" } else { "<-" },
			reason_kind_label(u64_field(e, "kind")),
			f64_field(e, "score"),
			short_id(if outgoing { str_field(e, "to") } else { from }),
			truncate(str_field(e, "text"), 80),
		);
	}
}

// Routed first for the same reason as forget: a serving daemon's graph is newer
// than anything this process can load, so a local read would print a stale
// thought — and stale evidence is the defect one step down from a lost write.
pub(super) async fn cmd_get(cfg: &crate::config::Config, id: &str) {
	match route("query", serde_json::json!({"id": id})).await {
		Routed::Done(v) => return print_detail(&v),
		Routed::Refused(e) => return eprintln!("{e}"),
		Routed::NoDaemon => {}
	}
	let g = load_graph(cfg);
	match entity_detail_by_id(&g, id) {
		Some(detail) => print_detail(&detail),
		None => eprintln!("thought not found: {id}"),
	}
}

pub(super) fn cmd_list(cfg: &crate::config::Config) {
	let g: GraphGnn = load_graph(cfg);
	print_kern(&g.root, &g, 0);
}

fn print_forget(id: &str, removed: u64) {
	println!("forgot {}  removed {} edges", short_id(id), removed);
}

// Routed first: while a daemon serves, its in-memory graph is newer than
// anything this process can load, so a local forget would delete from a stale
// copy and report a stale edge count.
pub(super) async fn cmd_forget(cfg: &crate::config::Config, id: &str) {
	match route("forget", serde_json::json!({"id": id})).await {
		Routed::Done(v) => return print_forget(id, u64_field(&v, "removed_edges")),
		Routed::Refused(e) => return eprintln!("{e}"),
		Routed::NoDaemon => {}
	}
	with_graph(cfg, |g| match forget_entity(g, id) {
		Ok(removed) => print_forget(id, removed as u64),
		Err(e) => eprintln!("{e}: {id}"),
	});
}

pub(crate) fn forget_entity(g: &mut GraphGnn, id: &str) -> Result<usize, &'static str> {
	let (thought, kern_id) = find_entity(g, id).ok_or("thought not found")?;
	// A remote Fact is a peer's assertion, not durable local knowledge — forgettable.
	if thought.is_fact() && !crate::base::merge::is_remote_kern_id(&kern_id) {
		return Err("cannot forget a fact");
	}
	let edges_before = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
	remove_entity(g, &kern_id, id);
	let edges_after = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
	// saturating: remove_entity only drops edges, never adds — guard against underflow.
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
	let g = load_graph(cfg);
	let (from_t, _) = match find_entity(&g, from) {
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

	match link_and_persist(g, cfg, from, to, reason_text, reason_embed) {
		Ok((rid, score)) => println!(
			"linked {} -> {}  edge={}  score={:.4}",
			short_id(from),
			short_id(to),
			short_id(&rid),
			score,
		),
		Err(e) => eprintln!("{e}"),
	}
}

// Takes the loaded graph by value so the stale-graph case is reachable from a
// test: the race this guards against is a commit landing between the load and
// the flush, which nothing outside `cmd_link` can interleave while the load is
// buried inside it.
fn link_and_persist(
	mut g: GraphGnn,
	cfg: &crate::config::Config,
	from: &str,
	to: &str,
	reason_text: String,
	reason_embed: Option<Vec<f32>>,
) -> Result<(String, f64), String> {
	let linked = link_entities(&mut g, from, to, reason_text, reason_embed, 1.0)?;
	// Guarded, not `save_graph_unguarded`: this command holds no writer lock, so
	// a daemon can commit between our load and our flush. The unguarded path
	// writes the whole kern map with no epoch check and drops that commit.
	let g = std::sync::Arc::new(parking_lot::RwLock::new(g));
	super::save_graph_guarded(&g, cfg);
	Ok(linked)
}

// `score` is the assertion's strength, NOT cosine(from, to): a deliberate link
// exists precisely to connect what content similarity cannot, so scoring it by
// endpoint similarity guarantees the edge is weakest exactly where it is the
// only evidence. Callers pass their source's confidence (user 1.0, agent 0.95).
pub(crate) fn link_entities(
	g: &mut GraphGnn,
	from: &str,
	to: &str,
	reason_text: String,
	reason_embed: Option<Vec<f32>>,
	score: f64,
) -> Result<(String, f64), String> {
	let (from_t, from_kern_id) =
		find_entity(g, from).ok_or_else(|| format!("from thought not found: {from}"))?;
	let (to_t, _) = find_entity(g, to).ok_or_else(|| format!("to thought not found: {to}"))?;

	let vec = link_vector(reason_embed, &from_t.vector, &to_t.vector);
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

	let kern = g.kerns.get_mut(&from_kern_id).ok_or_else(|| {
		format!(
			"link failed: kern {} no longer present",
			short_id(&from_kern_id)
		)
	})?;
	add_reason(kern, r);
	Ok((rid, score))
}

fn link_vector(reason_embed: Option<Vec<f32>>, from_vec: &[f32], to_vec: &[f32]) -> Vec<f32> {
	reason_embed.unwrap_or_else(|| average_vec(from_vec, to_vec))
}

fn print_degrade(id: &str, decayed: u64, removed: u64) {
	println!(
		"degraded {}  decayed {} edges, removed {} below threshold",
		short_id(id),
		decayed,
		removed,
	);
}

pub(super) async fn cmd_degrade(cfg: &crate::config::Config, id: &str) {
	match route("degrade", serde_json::json!({"query_id": id})).await {
		Routed::Done(v) => {
			return print_degrade(
				id,
				u64_field(&v, "decayed_edges"),
				u64_field(&v, "removed_edges"),
			)
		}
		Routed::Refused(e) => return eprintln!("{e}"),
		Routed::NoDaemon => {}
	}
	with_graph(cfg, |g| {
		let (_, kern_id) = match find_entity(g, id) {
			Some(pair) => pair,
			None => {
				eprintln!("thought not found: {id}");
				return;
			}
		};
		let (decayed, removed) = degrade_entity_reasons(g, &kern_id, id);
		print_degrade(id, decayed as u64, removed as u64);
	});
}

pub(crate) fn degrade_entity_reasons(g: &mut GraphGnn, kern_id: &str, id: &str) -> (usize, usize) {
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
		} else {
			let lamport = g.bump_lamport();
			let producer = g.network_id.clone();
			if let Some(kern) = g.kerns.get_mut(kern_id) {
				if let Some(r) = kern.reasons.get_mut(rid) {
					r.score -= decay;
					r.score_lamport = lamport;
					r.score_producer = producer.clone();
					let lww_value =
						bincode::serde::encode_to_vec(r.score, bincode::config::standard()).unwrap_or_default();
					g.push_delta(crate::base::graph::PendingDelta {
						object_id: rid.clone(),
						target: 2,
						replica: String::new(),
						value: 0,
						lamport,
						producer,
						lww_value,
					});
				}
			}
		}
		decayed += 1;
	}
	(decayed, removed)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Kern, Reason};

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
		// BASE=0.15 pushes a->c (0.0) below the 0.05 floor; a->b (1.0) merely decays.
		add_reason(&mut k, edge("a", "b", 1.0));
		add_reason(&mut k, edge("a", "c", 0.0));
		g.kerns.insert("kx".into(), k);

		let (decayed, removed) = degrade_entity_reasons(&mut g, "kx", "a");

		assert_eq!(decayed, 2, "both incident edges visited");
		assert_eq!(removed, 1, "the sub-threshold edge is reaped");

		let kern = g.kerns.get("kx").expect("kern present");
		assert_eq!(kern.reasons.len(), 1, "only the healthy edge remains");
		let survivor = kern.reasons.get("a->b").expect("a->b survives");
		assert!(
			survivor.score < 1.0,
			"survivor was decayed, not left untouched"
		);
		assert!(
			survivor.score >= DEGRADE_MIN_THRESHOLD,
			"survivor stays above the floor"
		);
	}

	// `kern link` takes no writer lock, so a daemon can commit between its load
	// and its flush. The unguarded save writes the whole kern map with no epoch
	// check, so that commit vanishes — the last half of item 9 that needed no
	// auth to close.
	#[test]
	fn a_link_racing_an_external_commit_keeps_both() {
		use parking_lot::RwLock;
		use std::sync::Arc;

		use crate::base::types::{mk_entity, EntityKind};

		let dir = tempfile::tempdir().unwrap();
		let cfg = crate::config::Config {
			data_dir: dir.path().to_string_lossy().into_owned(),
			..Default::default()
		};

		let g = Arc::new(RwLock::new(crate::commands::load_graph(&cfg)));
		let root_id = g.read().root.id.clone();

		let mut own = Kern::new("link-kern", &root_id);
		for id in ["a", "b"] {
			own
				.entities
				.insert(id.into(), mk_entity(id, id, 1.0, EntityKind::Claim));
		}
		g.write().kerns.insert("link-kern".into(), own);
		crate::commands::save_graph_guarded(&g, &cfg);

		// What `cmd_link` holds: loaded now, flushed only after the daemon commits.
		// That staleness is the defect — a graph loaded fresh would already carry
		// the other writer's kern and write it back by accident.
		let stale = crate::commands::load_graph(&cfg);
		crate::test_support::commit_extra_kern_via_store(&g, Kern::new("daemon-kern", &root_id));
		drop(g);

		let linked = link_and_persist(stale, &cfg, "a", "b", "because".into(), None);
		assert!(linked.is_ok(), "the link itself applies: {linked:?}");

		let disk = crate::commands::load_graph(&cfg);
		assert!(
			disk.loaded("daemon-kern").is_some(),
			"the concurrent writer's kern survived the link's flush"
		);
		let kern = disk.kerns.get("link-kern").expect("our own kern persisted");
		assert!(
			kern.reasons.values().any(|r| r.from == "a" && r.to == "b"),
			"the edge we just wrote is on disk too"
		);
	}

	#[test]
	fn degrade_on_unknown_kern_is_a_noop() {
		let mut g = GraphGnn::new();
		let (decayed, removed) = degrade_entity_reasons(&mut g, "missing", "a");
		assert_eq!((decayed, removed), (0, 0));
	}

	#[test]
	fn link_vector_prefers_the_reason_embedding() {
		let v = link_vector(
			Some(vec![1.0, 2.0, 3.0]),
			&[0.0, 0.0, 0.0],
			&[9.0, 9.0, 9.0],
		);
		assert_eq!(
			v,
			vec![1.0, 2.0, 3.0],
			"an embedded reason wins over the midpoint"
		);
	}

	#[test]
	fn link_vector_falls_back_to_endpoint_midpoint() {
		let v = link_vector(None, &[0.0, 2.0], &[4.0, 6.0]);
		assert_eq!(
			v,
			vec![2.0, 4.0],
			"no embedding -> midpoint of the two endpoints"
		);
	}

	use crate::base::types::EntityKind;

	fn ent(id: &str, kind: EntityKind) -> Entity {
		Entity {
			id: id.into(),
			kind,
			..Default::default()
		}
	}

	fn graph_with(entities: &[(&str, EntityKind)], edges: &[(&str, &str)]) -> GraphGnn {
		graph_in("kx", entities, edges)
	}

	fn graph_in(kern_id: &str, entities: &[(&str, EntityKind)], edges: &[(&str, &str)]) -> GraphGnn {
		let mut g = GraphGnn::new();
		let mut k = Kern::new(kern_id, "");
		for (id, kind) in entities {
			k.entities.insert((*id).into(), ent(id, *kind));
		}
		for (from, to) in edges {
			add_reason(&mut k, edge(from, to, 1.0));
		}
		g.register(k);
		g
	}

	#[test]
	fn forget_removes_thought_and_reports_edge_delta() {
		let mut g = graph_with(
			&[
				("a", EntityKind::Claim),
				("b", EntityKind::Claim),
				("c", EntityKind::Claim),
			],
			&[("a", "b"), ("a", "c")],
		);
		let removed = forget_entity(&mut g, "a").expect("non-fact forget succeeds");
		assert_eq!(removed, 2, "both incident edges went with a");
		let kern = g.kerns.get("kx").expect("kern present");
		assert!(!kern.entities.contains_key("a"), "a is gone from the kern");
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

	// Without this the operator has no way to remove a peer-pinned Fact by hand.
	#[test]
	fn forget_allows_a_remote_fact() {
		let mut g = graph_in("remote-evilnet-k1", &[("f", EntityKind::Fact)], &[]);
		assert_eq!(
			forget_entity(&mut g, "f"),
			Ok(0),
			"a remote Fact must be forgettable"
		);
		assert!(
			!g.kerns
				.get("remote-evilnet-k1")
				.unwrap()
				.entities
				.contains_key("f"),
			"the remote fact is actually gone, not just reported gone"
		);
	}

	#[test]
	fn forget_unknown_id_is_rejected_not_panicked() {
		let mut g = graph_with(&[("a", EntityKind::Claim)], &[]);
		assert_eq!(forget_entity(&mut g, "nope"), Err("thought not found"));
	}

	// Proves the printer, not the lookup: both `kern get` paths hand it this shape,
	// so the labels and the edge direction must come back out of the JSON intact.
	#[test]
	fn detail_json_carries_everything_the_get_printer_needs() {
		let mut g = graph_with(
			&[("a", EntityKind::Question), ("b", EntityKind::Claim)],
			&[("a", "b")],
		);
		g.kerns
			.get_mut("kx")
			.unwrap()
			.entities
			.get_mut("a")
			.unwrap()
			.set_text("the question".into());
		let v = entity_detail_by_id(&g, "a").expect("a resolves");

		assert_eq!(entity_kind_label(u64_field(&v, "kind")), "Question");
		assert_eq!(str_field(&v, "text"), "the question");
		assert_eq!(str_field(&v, "kern"), "kx");
		let edges = array_field(&v, "edges");
		assert_eq!(edges.len(), 1);
		assert_eq!(str_field(&edges[0], "from"), "a", "edge points outward");
		assert_eq!(
			reason_kind_label(u64_field(&edges[0], "kind")),
			"Similarity"
		);
		assert!(entity_detail_by_id(&g, "nope").is_none());
	}
}
