//! "Memory of the day": a condensed digest of one compacted day, synthesized
//! from the journal archive (what happened — sessions, plans, tool use) and the
//! kern graph (what was learned — entities the day touched), rendered as an
//! Obsidian markdown note. Optional, gated by `[journal] obsidian_export`.
//!
//! Pure/seam-isolated so it is unit-testable without Ollama: the LLM is just a
//! `&dyn Fn(&str) -> String` (the kern-wide `LlmFunc` shape), and the graph
//! lookup is a plain read. The compactor (`ingest::compactor::run`) wires these
//! to the live `History`, `SharedGraph`, and Ollama client.

use std::path::{Path, PathBuf};

use journal::{Entry, Filter, History, Kind};

use crate::base::graph::GraphGnn;

/// Curated kinds for a day digest — sessions, plans, goals, tool use, and entity
/// touches. Excludes `Log` (tracing noise) and low-signal RPC chatter.
fn is_curated(kind: &Kind) -> bool {
	matches!(
		kind,
		Kind::ForkOpen { .. }
			| Kind::ForkResume { .. }
			| Kind::ForkClose { .. }
			| Kind::PlanProposal
			| Kind::PlanStep
			| Kind::Goal
			| Kind::GoalSnapshot
			| Kind::Milestone
			| Kind::ToolCall
			| Kind::EntityTouched
	)
}

/// Curated journal events for archive `day` ("YYYY-MM-DD"), ascending by ts.
pub(crate) fn gather_day_journal(history: &History, day: &str) -> anyhow::Result<Vec<Entry>> {
	let mut rows = history.query(Filter {
		day: Some(day.to_string()),
		..Filter::default()
	})?;
	rows.retain(|e| is_curated(&e.kind));
	Ok(rows)
}

/// Fork ids opened during the day (the day's sessions), first-seen order.
fn session_ids(events: &[Entry]) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for e in events {
		if let Kind::ForkOpen { fork_id, .. } = &e.kind {
			if !out.contains(fork_id) {
				out.push(fork_id.clone());
			}
		}
	}
	out
}

/// Distinct entity_ids referenced by the day's `EntityTouched` events
/// (first-seen order).
fn touched_entity_ids(events: &[Entry]) -> Vec<String> {
	let mut seen = std::collections::HashSet::new();
	let mut out = Vec::new();
	for e in events {
		if matches!(e.kind, Kind::EntityTouched) {
			if let Some(id) = e.payload.get("entity_id").and_then(|v| v.as_str()) {
				if seen.insert(id.to_string()) {
					out.push(id.to_string());
				}
			}
		}
	}
	out
}

/// Resolve entity ids to `(id, kind, label)` by walking the graph; `label` is the
/// first statement (truncated to 80 chars). Missing ids are skipped.
pub(crate) fn resolve_entities(g: &GraphGnn, ids: &[String]) -> Vec<(String, String, String)> {
	let want: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
	let mut out = Vec::new();
	for kern in g.kerns.values() {
		for ent in kern.entities.values() {
			if want.contains(ent.id.as_str()) {
				let label: String = ent
					.statements
					.first()
					.map(|s| s.chars().take(80).collect())
					.unwrap_or_default();
				out.push((ent.id.clone(), ent.kind.as_str().to_string(), label));
			}
		}
	}
	out
}

/// Everything the renderer needs about one day.
pub(crate) struct DayInputs {
	pub day: String,
	pub sessions: Vec<String>,
	pub entities: Vec<(String, String, String)>,
	pub tool_calls: usize,
}

/// Build the day's inputs from the archive + graph (no LLM, no I/O beyond the
/// SQLite read). The compactor holds the graph read lock only for this call,
/// then releases it before the (slow) LLM render.
pub(crate) fn build_day_inputs(
	history: &History,
	graph: &GraphGnn,
	day: &str,
) -> anyhow::Result<DayInputs> {
	let events = gather_day_journal(history, day)?;
	let sessions = session_ids(&events);
	let entities = resolve_entities(graph, &touched_entity_ids(&events));
	let tool_calls = events
		.iter()
		.filter(|e| matches!(e.kind, Kind::ToolCall))
		.count();
	Ok(DayInputs {
		day: day.to_string(),
		sessions,
		entities,
		tool_calls,
	})
}

fn build_prompt(i: &DayInputs) -> String {
	let mut s = format!(
		"Summarize the day {} as a short highlight (2-4 sentences). Be concrete.\nSessions: {}\nTool calls: {}\nKey knowledge touched:\n",
		i.day,
		if i.sessions.is_empty() { "none".into() } else { i.sessions.join(", ") },
		i.tool_calls,
	);
	for (_id, kind, label) in &i.entities {
		s.push_str(&format!("- [{kind}] {label}\n"));
	}
	s
}

/// Render the daily-note markdown. Returns `None` when the LLM yields nothing
/// (empty/whitespace), so the caller can defer and retry on a later pass.
pub(crate) fn render_markdown(i: &DayInputs, llm: &dyn Fn(&str) -> String) -> Option<String> {
	let highlight = llm(&build_prompt(i));
	if highlight.trim().is_empty() {
		return None;
	}
	let mut md = format!("# {}\n\n{}\n\n", i.day, highlight.trim());
	if !i.sessions.is_empty() {
		md.push_str("## Sessions\n");
		for s in &i.sessions {
			md.push_str(&format!("- {s}\n"));
		}
		md.push('\n');
	}
	if !i.entities.is_empty() {
		md.push_str("## Knowledge\n");
		for (_id, kind, label) in &i.entities {
			md.push_str(&format!("- [{kind}] [[{label}]]\n"));
		}
		md.push('\n');
	}
	md.push_str(&format!("> tool calls: {}\n", i.tool_calls));
	Some(md)
}

/// Write the daily note under `<vault>/YYYY/MM/YYYY-MM-DD.md` (dirs created;
/// overwrites for an idempotent re-render). Returns the written path.
pub(crate) fn write_day_note(vault: &Path, day: &str, contents: &str) -> std::io::Result<PathBuf> {
	let mut it = day.splitn(3, '-');
	let (y, m) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
	let dir = vault.join(y).join(m);
	std::fs::create_dir_all(&dir)?;
	let path = dir.join(format!("{day}.md"));
	std::fs::write(&path, contents)?;
	Ok(path)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, EntityKind};

	#[test]
	fn gather_day_journal_keeps_curated_excludes_log() {
		let h = History::open_in_memory().unwrap();
		h.bulk_insert(&[
			Entry::new(
				Kind::ForkOpen {
					fork_id: "f".into(),
					parent: None,
				},
				"f",
				serde_json::json!({ "fork_id": "f" }),
			),
			Entry::new(Kind::Log, "noise", serde_json::Value::Null),
			Entry::new(
				Kind::Milestone,
				"m",
				serde_json::json!({ "text": "shipped" }),
			),
		])
		.unwrap();
		// Entries are stamped now() -> stored under today's archive day.
		let events = gather_day_journal(&h, &journal::today()).unwrap();
		assert!(
			events.iter().all(|e| !matches!(e.kind, Kind::Log)),
			"Log excluded"
		);
		assert_eq!(events.len(), 2, "ForkOpen + Milestone kept");
	}

	#[test]
	fn touched_and_session_ids_dedupe_first_seen() {
		let evs = vec![
			Entry::new(
				Kind::ForkOpen {
					fork_id: "a".into(),
					parent: None,
				},
				"a",
				serde_json::json!({}),
			),
			Entry::new(
				Kind::ForkOpen {
					fork_id: "a".into(),
					parent: None,
				},
				"a",
				serde_json::json!({}),
			),
			Entry::new(
				Kind::EntityTouched,
				"e1",
				serde_json::json!({ "entity_id": "e1" }),
			),
			Entry::new(
				Kind::EntityTouched,
				"e1",
				serde_json::json!({ "entity_id": "e1" }),
			),
			Entry::new(
				Kind::EntityTouched,
				"e2",
				serde_json::json!({ "entity_id": "e2" }),
			),
		];
		assert_eq!(session_ids(&evs), vec!["a".to_string()]);
		assert_eq!(
			touched_entity_ids(&evs),
			vec!["e1".to_string(), "e2".to_string()]
		);
	}

	#[test]
	fn resolve_entities_reads_label_and_kind_from_graph() {
		let mut g = GraphGnn::new();
		let root_id = g.root.id.clone();
		let ent = Entity {
			id: "e1".into(),
			kind: EntityKind::Fact,
			statements: vec!["kern is standalone".into()],
			..Default::default()
		};
		g.kerns
			.get_mut(&root_id)
			.unwrap()
			.entities
			.insert("e1".into(), ent);

		let resolved = resolve_entities(&g, &["e1".to_string(), "missing".to_string()]);
		assert_eq!(
			resolved,
			vec![(
				"e1".to_string(),
				"fact".to_string(),
				"kern is standalone".to_string()
			)],
			"resolves present id; skips missing",
		);
	}

	#[test]
	fn render_markdown_has_highlight_sessions_and_links() {
		let inputs = DayInputs {
			day: "2026-06-12".into(),
			sessions: vec!["fork-a".into()],
			entities: vec![("e1".into(), "fact".into(), "kern is standalone".into())],
			tool_calls: 3,
		};
		let md = render_markdown(&inputs, &|_p: &str| {
			"Big day: shipped the compactor.".to_string()
		})
		.unwrap();
		assert!(md.starts_with("# 2026-06-12"), "dated H1");
		assert!(
			md.contains("Big day: shipped the compactor."),
			"LLM highlight"
		);
		assert!(md.contains("[[kern is standalone]]"), "entity wikilink");
		assert!(md.contains("- fork-a"), "session listed");
		assert!(md.contains("tool calls: 3"));
	}

	#[test]
	fn render_markdown_defers_when_llm_empty() {
		let inputs = DayInputs {
			day: "2026-06-12".into(),
			sessions: vec![],
			entities: vec![],
			tool_calls: 0,
		};
		assert!(
			render_markdown(&inputs, &|_p: &str| "   ".to_string()).is_none(),
			"empty LLM -> defer"
		);
	}

	#[test]
	fn writes_note_to_year_month_day_path() {
		let dir = tempfile::tempdir().unwrap();
		let path = write_day_note(dir.path(), "2026-06-12", "# 2026-06-12\n\nhi\n").unwrap();
		assert_eq!(
			path,
			dir.path().join("2026").join("06").join("2026-06-12.md")
		);
		assert_eq!(
			std::fs::read_to_string(&path).unwrap(),
			"# 2026-06-12\n\nhi\n"
		);
	}
}
