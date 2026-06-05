use crate::base::constants::{DEGRADE_DECAY_BASE, DEGRADE_DECAY_POW, DEGRADE_MIN_THRESHOLD};
use crate::base::graph::GraphGnn;
use crate::base::math::{average_vec, cosine, reason_id};
use crate::base::reason::{add_reason, remove_reason, remove_entity};
use crate::base::search::find_entity;
use crate::base::types::{Reason, ReasonKind};
use crate::base::util::{explain_relationship_prompt, short_id, truncate};

use super::{build_llm, find_entity_by_prefix, load_graph, print_kern, save_graph, with_graph};

pub(super) fn cmd_get(cfg: &crate::config::Config, id: &str) {
	let g = load_graph(cfg);
	let (thought, kern_id) = match find_entity_by_prefix(&g, id) {
		Some(pair) => pair,
		None => {
			// Lazy rehydrate: a thought evicted by stigmergy GC was spilled to
			// the detached cold tier before being dropped from the hot graph.
			let cold_dir = std::path::PathBuf::from(&cfg.data_dir).join("cold");
			if let Some(e) = crate::base::cold::get(&cold_dir, id) {
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
	with_graph(cfg, |g| {
		let (thought, kern_id) = match find_entity(g, id) {
			Some(pair) => pair,
			None => {
				eprintln!("thought not found: {id}");
				return;
			}
		};
		if thought.is_fact() {
			eprintln!("cannot forget a fact");
			return;
		}
		let edges_before = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
		remove_entity(g, &kern_id, id);
		let edges_after = g.kerns.get(&kern_id).map(|k| k.reasons.len()).unwrap_or(0);
		println!(
			"forgot {}  removed {} edges",
			short_id(id),
			edges_before - edges_after
		);
	});
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

	let llm_client = build_llm(
		embed_url,
		embed_model,
		&cfg.embed.key,
		reason_url,
		reason_model,
		cfg.reason_key(),
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

	let vec = if !reason_text.is_empty() {
		llm_client
			.embed(&reason_text)
			.await
			.unwrap_or_else(|_| average_vec(&from_t.vector, &to_t.vector))
	} else {
		average_vec(&from_t.vector, &to_t.vector)
	};

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

	if let Some(kern) = g.kerns.get_mut(&from_kern_id) {
		add_reason(kern, r);
	}
	save_graph(&g);

	println!(
		"linked {} -> {}  edge={}  score={:.4}",
		short_id(from),
		short_id(to),
		short_id(&rid),
		score,
	);
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

		let rids: Vec<String> = if let Some(kern) = g.kerns.get(&kern_id) {
			crate::base::reason::collect_reason_ids(kern, id)
		} else {
			Vec::new()
		};

		let mut decayed = 0usize;
		let mut removed = 0usize;
		for (i, rid) in rids.iter().enumerate() {
			let decay = DEGRADE_DECAY_BASE * DEGRADE_DECAY_POW.powi(i as i32);

			let should_remove = if let Some(kern) = g.kerns.get(&kern_id) {
				kern.reasons
					.get(rid)
					.map(|r| r.score - decay < DEGRADE_MIN_THRESHOLD)
					.unwrap_or(false)
			} else {
				false
			};

			if should_remove {
				if let Some(kern) = g.kerns.get_mut(&kern_id) {
					remove_reason(kern, rid);
				}
				removed += 1;
			} else if let Some(kern) = g.kerns.get_mut(&kern_id) {
				if let Some(r) = kern.reasons.get_mut(rid) {
					r.score -= decay;
				}
			}
			decayed += 1;
		}
		println!(
			"degraded {}  decayed {} edges, removed {} below threshold",
			short_id(id),
			decayed,
			removed,
		);
	});
}
