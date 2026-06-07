//! Recall digest: a markdown snapshot of the kern's purpose plus its
//! hottest thoughts, written to disk for the Claude-Code SessionStart hook
//! to inject. Pure builder + a thin file writer; no live query path.

use crate::base::graph::GraphGnn;
use crate::base::types::{Entity, EntityKind, EntityStatus};

/// Rough token estimate for budgeting: ~4 chars/token (OpenAI/BGE-class
/// tokenizers average close to this for English). Deliberately cheap — the
/// digest only needs an approximate budget, not exact tokenization.
fn est_tokens(s: &str) -> usize {
	s.len() / 4 + 1
}

/// Normalized key for cheap near-duplicate detection: lowercase, whitespace
/// collapsed, capped length. Two claims that restate the same fact collapse to
/// the same key so only the first (hottest) is kept.
fn dedup_key(s: &str) -> String {
	let norm: String = s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
	norm.chars().take(80).collect()
}

/// Render the digest markdown: purpose header + the highest-value active
/// claims, best first.
///
/// Curation (card #49):
/// - **Ranking** is `heat * conf_mean`, not heat alone, so a hot but
///   low-confidence claim sinks below a warm, well-corroborated one.
/// - **`min_trust`** is a posterior-confidence floor (`Entity::conf_mean`):
///   claims below it are quarantined out of the digest. The digest is replayed
///   into every future session — the persistent re-injection surface for
///   memory-poisoning — so gating keeps low-trust / repeatedly-contradicted
///   claims off it. Pass `0.0` to disable the gate.
/// - **`token_budget`** caps the body by an approximate token count (context
///   rot: attention degrades with length), trimmed greedily; `k` remains a hard
///   upper bound on item count. Pass `0` to disable the token cap.
/// - **Diversity**: near-duplicate claims (same normalized text) are skipped so
///   restatements don't waste the budget.
pub fn build_digest(graph: &GraphGnn, k: usize, min_trust: f64, token_budget: usize) -> String {
	let mut out = String::from("# kern memory\n\n");
	let anchors: Vec<String> = crate::base::accept::root_anchor_ids(graph)
		.iter()
		.filter_map(|cid| graph.loaded(cid))
		.map(|c| c.anchor_text.clone())
		.collect();
	if !anchors.is_empty() {
		out.push_str("Anchors: ");
		out.push_str(&anchors.join(", "));
		out.push_str("\n\n");
	}

	let mut ranked: Vec<(&Entity, f64)> = graph
		.kerns
		.values()
		.flat_map(|kern| kern.entities.values())
		.filter(|e| {
			matches!(e.status, EntityStatus::Active)
				&& !matches!(e.kind, EntityKind::Document | EntityKind::Question)
				&& e.statements.first().is_some_and(|s| !s.trim().is_empty())
				&& e.conf_mean() >= min_trust
		})
		.map(|e| (e, e.heat as f64 * e.conf_mean()))
		.collect();
	ranked.sort_by(|a, b| {
		b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
	});

	// Greedy select: cap by item count (k) AND token budget, skipping
	// near-duplicate restatements.
	let mut bullets: Vec<&str> = Vec::new();
	let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
	let mut tokens = 0usize;
	for (e, _) in ranked {
		if bullets.len() >= k {
			break;
		}
		let Some(s) = e.statements.first().map(|s| s.trim()) else {
			continue;
		};
		if !seen.insert(dedup_key(s)) {
			continue; // near-duplicate of an already-selected claim
		}
		let t = est_tokens(s);
		if token_budget > 0 && !bullets.is_empty() && tokens + t > token_budget {
			break;
		}
		tokens += t;
		bullets.push(s);
	}

	if !bullets.is_empty() {
		out.push_str("## What I know\n\n");
		for s in bullets {
			out.push_str("- ");
			out.push_str(s);
			out.push('\n');
		}
	}
	out
}

/// Render and write the digest to `path`, creating parent dirs. Best effort.
pub fn write_digest(
	graph: &GraphGnn,
	path: &std::path::Path,
	k: usize,
	min_trust: f64,
	token_budget: usize,
) {
	if let Some(parent) = path.parent() {
		let _ = std::fs::create_dir_all(parent);
	}
	if let Err(e) = std::fs::write(path, build_digest(graph, k, min_trust, token_budget)) {
		tracing::warn!(target: "kern.digest", path = %path.display(), error = %e, "digest write failed");
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::graph::GraphGnn;
	use crate::base::types::{mk_entity, EntityKind};

	#[test]
	fn digest_has_anchor_and_hottest_first_capped() {
		let mut g = GraphGnn::default();
		// A named anchor (root child) should surface in the digest header.
		crate::base::accept::add_anchor(&mut g, "durable facts", vec![1.0, 0.0, 0.0]);
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities.insert("a".into(), mk_entity("a", "cold fact", 0.1, EntityKind::Claim));
		kern.entities.insert("b".into(), mk_entity("b", "hot fact", 9.0, EntityKind::Claim));

		let md = build_digest(&g, 1, 0.0, 0);
		assert!(md.contains("Anchors: durable facts"), "anchor present in header");
		assert!(md.contains("hot fact"), "hottest included");
		assert!(!md.contains("cold fact"), "capped at k=1");
	}

	#[test]
	fn documents_are_excluded_claims_kept() {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities.insert("doc".into(), mk_entity("doc", "raw document chunk", 9.0, EntityKind::Document));
		kern.entities.insert("clm".into(), mk_entity("clm", "a distilled claim", 0.5, EntityKind::Claim));

		let md = build_digest(&g, 10, 0.0, 0);
		assert!(md.contains("a distilled claim"), "claim kept");
		assert!(!md.contains("raw document chunk"), "document excluded even though hotter");
	}

	#[test]
	fn empty_graph_yields_header_only() {
		let g = GraphGnn::default();
		let md = build_digest(&g, 10, 0.0, 0);
		assert!(md.contains("# kern memory"));
	}

	#[test]
	fn low_trust_claim_quarantined_even_when_hottest() {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		// Hottest entity, but repeatedly contradicted → low posterior trust.
		let mut poisoned = mk_entity("p", "poisoned hot claim", 99.0, EntityKind::Claim);
		poisoned.conf_alpha = 1.0;
		poisoned.conf_beta = 9.0; // conf_mean = 0.1
		poisoned.refresh_score();
		kern.entities.insert("p".into(), poisoned);
		kern.entities
			.insert("t".into(), mk_entity("t", "trusted cool claim", 0.5, EntityKind::Claim));

		// Gate at 0.35: poisoned (0.1) quarantined despite being hottest;
		// trusted (mk_entity mean 0.667) survives.
		let gated = build_digest(&g, 10, 0.35, 0);
		assert!(!gated.contains("poisoned hot claim"), "low-trust claim quarantined");
		assert!(gated.contains("trusted cool claim"), "trusted claim kept");

		// Gate off: poisoned claim re-injected (hottest first).
		let ungated = build_digest(&g, 10, 0.0, 0);
		assert!(ungated.contains("poisoned hot claim"), "gate off → re-injected");
	}

	#[test]
	fn token_budget_trims_body_greedily() {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		// Three ~40-char claims; hotter = earlier. Budget admits ~the first one.
		kern.entities.insert(
			"a".into(),
			mk_entity("a", "alpha claim with some length to it here", 9.0, EntityKind::Claim),
		);
		kern.entities.insert(
			"b".into(),
			mk_entity("b", "bravo claim with some length to it here", 8.0, EntityKind::Claim),
		);
		kern.entities.insert(
			"c".into(),
			mk_entity("c", "charlie claim with some length here too", 7.0, EntityKind::Claim),
		);
		// ~10 tokens budget: first bullet always admitted, later ones trimmed.
		let md = build_digest(&g, 10, 0.0, 10);
		assert!(md.contains("alpha claim"), "hottest within budget kept");
		assert!(!md.contains("charlie claim"), "over-budget claim trimmed");
	}

	#[test]
	fn near_duplicate_claims_are_skipped() {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities
			.insert("a".into(), mk_entity("a", "The build uses cargo nextest", 9.0, EntityKind::Claim));
		// Same fact, different casing/spacing → same dedup key → skipped.
		kern.entities
			.insert("b".into(), mk_entity("b", "the build   uses CARGO nextest", 8.0, EntityKind::Claim));
		kern.entities
			.insert("c".into(), mk_entity("c", "Deploys run on fridays", 7.0, EntityKind::Claim));

		let md = build_digest(&g, 10, 0.0, 0);
		let bullets = md.matches("\n- ").count();
		assert_eq!(bullets, 2, "near-duplicate collapsed to one bullet");
		assert!(md.contains("Deploys run on fridays"));
	}

	#[test]
	fn ranks_by_heat_times_confidence() {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		// Hotter but low-confidence vs cooler but high-confidence.
		let mut hot_lowconf = mk_entity("h", "hot but shaky", 10.0, EntityKind::Claim);
		hot_lowconf.conf_alpha = 1.0;
		hot_lowconf.conf_beta = 3.0; // conf_mean 0.25 → score 2.5
		hot_lowconf.refresh_score();
		let mut warm_trusted = mk_entity("w", "warm and solid", 5.0, EntityKind::Claim);
		warm_trusted.conf_alpha = 9.0;
		warm_trusted.conf_beta = 1.0; // conf_mean 0.9 → score 4.5
		warm_trusted.refresh_score();
		kern.entities.insert("h".into(), hot_lowconf);
		kern.entities.insert("w".into(), warm_trusted);

		// k=1, no gate: heat*conf picks the warm trusted claim over the hot shaky one.
		let md = build_digest(&g, 1, 0.0, 0);
		assert!(md.contains("warm and solid"), "heat*conf ranks trusted above hot-but-shaky");
		assert!(!md.contains("hot but shaky"));
	}
}
