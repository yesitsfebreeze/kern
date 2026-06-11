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

	// Append the connections section (capped at 1/3 of the remaining budget so it
	// doesn't crowd out the entity bullets).
	let conn_budget = if token_budget > 0 {
		let used = est_tokens(&out);
		(token_budget.saturating_sub(used)) / 3
	} else {
		500 // default connection budget when no cap
	};
	out.push_str(&build_connections(graph, conn_budget));

	out
}

/// Build the `## Connections` markdown section: the top enriched relationship
/// edges (the specific logical connections between entities), ranked by the
/// heat×confidence of their from-entity, deduped, and trimmed to `conn_budget`
/// approximate tokens. Only Similarity/Provenance/Ratification edges (the
/// semantically meaningful ones) are included — structural kinds like
/// Spawn/Supersedes carry no explanatory prose. Returns the section with its
/// leading newline + header, or an empty string when nothing qualifies/fits.
fn build_connections(graph: &GraphGnn, conn_budget: usize) -> String {
	if conn_budget == 0 {
		return String::new();
	}
	let mut conn_lines: Vec<String> = Vec::new();
	let mut conn_tokens = 0usize;
	let mut conn_seen: std::collections::HashSet<String> = std::collections::HashSet::new();

	// Build entity map once: id → (display_text, heat×conf). Avoids an
	// O(N×M) nested kerns scan per reason during scoring and formatting.
	let entity_cache: std::collections::HashMap<&str, (String, f64)> = graph
		.kerns
		.values()
		.flat_map(|k| k.entities.values())
		.map(|e| {
			let t = e.text();
			let display = match t.char_indices().nth(39) {
				Some((byte_pos, _)) => format!("{}…", &t[..byte_pos]),
				None => t,
			};
			(e.id.as_str(), (display, e.heat as f64 * e.conf_mean()))
		})
		.collect();

	let mut kern_reasons: Vec<_> = graph
		.kerns
		.values()
		.flat_map(|kern| kern.reasons.values())
		.filter(|r| r.is_enriched() && r.kind.is_semantic())
		.map(|r| {
			let heat_conf = entity_cache.get(r.from.as_str()).map(|(_, hc)| *hc).unwrap_or(0.0);
			(r, heat_conf)
		})
		.collect();
	kern_reasons.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

	for (r, _) in kern_reasons {
		let from_text = entity_cache
			.get(r.from.as_str())
			.map(|(t, _)| t.as_str())
			.unwrap_or_else(|| &r.from[..8.min(r.from.len())]);
		let line = format!("{} → {}", from_text, r.text.trim());
		let key = dedup_key(&line);
		if !conn_seen.insert(key) {
			continue;
		}
		let t = est_tokens(&line);
		if !conn_lines.is_empty() && conn_tokens + t > conn_budget {
			break;
		}
		conn_tokens += t;
		conn_lines.push(line);
	}

	if conn_lines.is_empty() {
		return String::new();
	}
	let mut section = String::from("\n## Connections\n\n");
	for l in &conn_lines {
		section.push_str("- ");
		section.push_str(l);
		section.push('\n');
	}
	section
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
	use crate::base::reason::add_reason;
	use crate::base::types::{mk_entity, EntityKind, Reason, ReasonKind};

	fn graph_with_reason(kind: ReasonKind, text: &str) -> GraphGnn {
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities.insert("a".into(), mk_entity("a", "entity alpha", 9.0, EntityKind::Claim));
		kern.entities.insert("b".into(), mk_entity("b", "entity beta", 8.0, EntityKind::Claim));
		add_reason(kern, Reason {
			id: "a->b".into(),
			from: "a".into(),
			to: "b".into(),
			kind,
			text: text.to_string(),
			score: 0.9,
			..Default::default()
		});
		g
	}

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

	#[test]
	fn enriched_connections_appear_in_digest() {
		let g = graph_with_reason(ReasonKind::Similarity, "alpha and beta share the same indexing mechanism");
		let md = build_digest(&g, 10, 0.0, 0);
		assert!(md.contains("## Connections"), "connections section present");
		assert!(md.contains("alpha and beta share the same indexing mechanism"), "enriched reason text in digest");
	}

	#[test]
	fn unenriched_reasons_excluded_from_connections() {
		let g = graph_with_reason(ReasonKind::Similarity, "");
		let md = build_digest(&g, 10, 0.0, 0);
		assert!(!md.contains("## Connections"), "unenriched reason produces no connections section");
	}

	#[test]
	fn connection_entity_display_truncates_on_char_boundary() {
		// Regression: the connection-label cache sliced entity text at raw byte
		// 39, which panicked when a multibyte char straddled the boundary (e.g.
		// '→' at bytes 38..41). Build an entity whose 40th boundary lands mid-char.
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		// 38 ASCII bytes, then a 3-byte '→' spanning bytes 38..41.
		let text = "UDP multicast discovery works for kern→kern but not browser→kern.";
		kern.entities.insert("a".into(), mk_entity("a", text, 9.0, EntityKind::Claim));
		kern.entities.insert("b".into(), mk_entity("b", "entity beta", 8.0, EntityKind::Claim));
		add_reason(kern, Reason {
			id: "a->b".into(),
			from: "a".into(),
			to: "b".into(),
			kind: ReasonKind::Similarity,
			text: "shared discovery path".into(),
			score: 0.9,
			..Default::default()
		});
		// Must not panic on the char boundary; truncated label ends with the ellipsis.
		let md = build_digest(&g, 10, 0.0, 0);
		assert!(md.contains("## Connections"), "connection rendered without panic");
		assert!(md.contains('…'), "long entity label truncated with ellipsis");
	}

	#[test]
	fn supersedes_reasons_excluded_from_connections() {
		let g = graph_with_reason(ReasonKind::Supersedes, "superseded by a newer version");
		let md = build_digest(&g, 10, 0.0, 0);
		assert!(!md.contains("## Connections"), "Supersedes reason excluded from connections");
	}

	#[test]
	fn write_digest_creates_parent_dirs_and_writes_the_markdown() {
		let dir = tempfile::tempdir().unwrap();
		// A nested path so create_dir_all is exercised.
		let path = dir.path().join("a/b/digest.md");
		let mut g = GraphGnn::default();
		let root_id = g.root.id.clone();
		let kern = g.kerns.get_mut(&root_id).expect("root kern");
		kern.entities.insert("a".into(), mk_entity("a", "a written claim", 9.0, EntityKind::Claim));

		write_digest(&g, &path, 10, 0.0, 0);
		let contents = std::fs::read_to_string(&path).expect("digest file exists after write");
		assert!(contents.contains("# kern memory"), "header written");
		assert!(contents.contains("a written claim"), "claim body written");
		// Matches the pure builder exactly (writer is a thin wrapper).
		assert_eq!(contents, build_digest(&g, 10, 0.0, 0));

		// Second write overwrites cleanly (no append / leftover).
		write_digest(&g, &path, 10, 0.0, 0);
		assert_eq!(std::fs::read_to_string(&path).unwrap().matches("# kern memory").count(), 1);
	}
}
