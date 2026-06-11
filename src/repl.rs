use std::io::{self, BufRead, Write};
use std::sync::{Arc, RwLock};

use crate::base::graph::GraphGnn;
use crate::base::locks::{read_recovered, write_recovered};
use crate::base::util::{short_id, truncate};

pub async fn run(
	graph: Arc<RwLock<GraphGnn>>,
	worker: Arc<crate::ingest::Worker>,
	llm: crate::llm::Client,
	task_q: Option<Arc<crate::tick::queue::Queue>>,
	dedup_threshold: f64,
) {
	let stdin = io::stdin();
	let mut stdout = io::stdout();

	loop {
		print!("kern> ");
		stdout.flush().ok();

		let mut line = String::new();
		match stdin.lock().read_line(&mut line) {
			Ok(0) => break,
			Err(_) => break,
			_ => {}
		}

		let line = line.trim();
		if line.is_empty() {
			continue;
		}

		let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
		let rest = rest.trim();

		match cmd {
			"query" | "q" => do_query(&graph, &llm, rest).await,
			"ingest" => do_ingest(&graph, &worker, rest, dedup_threshold).await,
			"health" => do_health(&graph, &task_q),
			"list" => do_list(&graph),
			"pulse" => do_pulse(&graph, &task_q),
			"quit" | "exit" => break,
			"?" | "help" => print_help(),
			_ => println!("unknown command: {cmd}  (type ? for help)"),
		}
	}
}

async fn do_query(graph: &Arc<RwLock<GraphGnn>>, llm: &crate::llm::Client, text: &str) {
	if text.is_empty() {
		println!("usage: query <text>");
		return;
	}

	let vec = match llm.embed(text).await {
		Ok(v) => v,
		Err(e) => {
			println!("embed: {e}");
			return;
		}
	};

	let mode = crate::retrieval::seed::Mode::Hybrid;
	let llm_fn: crate::types::LlmFunc = std::sync::Arc::new(llm.complete_func());

	let rcfg = crate::config::RetrievalConfig::default();
	let result = {
		let g = read_recovered(graph);
		crate::retrieval::answer::query(&g, &rcfg, &vec, text, mode, Some(&llm_fn), None, None)
	};

	if result.entities.is_empty() {
		println!("no results");
		return;
	}
	for (i, st) in result.entities.iter().enumerate() {
		println!(
			"{}. [{:.4}] {}  {}",
			i + 1,
			st.score,
			short_id(&st.entity.id),
			truncate(&st.entity.text(), 120),
		);
	}
	if !result.answer.is_empty() {
		println!("--- Answer ---");
		println!("{}", result.answer);
	}
}

async fn do_ingest(
	_graph: &Arc<RwLock<GraphGnn>>,
	worker: &crate::ingest::Worker,
	text: &str,
	dedup_threshold: f64,
) {
	if text.is_empty() {
		println!("usage: ingest <text>");
		return;
	}

	let src = crate::base::types::Source::Session {
		session_id: "repl".to_string(),
		section: String::new(),
		title: String::new(),
	};
	let (conf, kind) = crate::base::math::clamp_confidence(1.0, "user");
	let outcome = worker
		.run(
			text.to_string(),
			src,
			kind,
			String::new(),
			conf,
			crate::ingest::Config {
				dedup_threshold,
				..Default::default()
			},
		)
		.await;
	println!(
		"ingested (status={} chunks={})",
		outcome.status.as_str(),
		outcome.total_chunks,
	);
}

fn do_health(graph: &Arc<RwLock<GraphGnn>>, task_q: &Option<Arc<crate::tick::queue::Queue>>) {
	let h = {
		let g = read_recovered(graph);
		crate::base::health::graph_health_stats(&g)
	};
	let queue_depth = task_q.as_ref().map(|q| q.pending_count()).unwrap_or(0);

	if h.anchors.is_empty() {
		println!("anchors:     (none)");
	} else {
		println!("anchors:     {}", h.anchors.join(", "));
	}
	println!("kerns:       {}", h.kerns);
	println!("thoughts:    {} (unnamed: {})", h.entities, h.unnamed);
	println!("reasons:     {}", h.reasons);
	println!("queue_depth: {queue_depth}");
}

fn do_list(graph: &Arc<RwLock<GraphGnn>>) {
	let g = read_recovered(graph);
	for line in list_lines(&g) {
		println!("{line}");
	}
}

/// Build the `list` command output: one `[id] text` line per entity, truncated
/// to the first 50 with a sentinel line, or a single "no entities" line on an
/// empty graph. Pure (no IO) so the cap and line format are unit-testable.
fn list_lines(g: &GraphGnn) -> Vec<String> {
	let mut lines = Vec::new();
	for k in g.all() {
		for t in k.entities.values() {
			lines.push(format!("[{}] {}", short_id(&t.id), truncate(&t.text(), 120)));
			if lines.len() >= 50 {
				lines.push("... (showing first 50)".to_string());
				return lines;
			}
		}
	}
	if lines.is_empty() {
		lines.push("no entities".to_string());
	}
	lines
}

fn do_pulse(graph: &Arc<RwLock<GraphGnn>>, task_q: &Option<Arc<crate::tick::queue::Queue>>) {
	let q = match task_q {
		Some(q) => q,
		None => {
			println!("task queue not available");
			return;
		}
	};
	let mut g = write_recovered(graph);
	let root_id = g.root.id.clone();
	crate::tick::pulse::pulse(q, &mut g, &root_id, 1.0);
	println!("pulsed");
}

fn print_help() {
	println!("{}", help_text());
}

/// The `help`/`?` command text. Pure so the command listing stays unit-testable.
fn help_text() -> &'static str {
	"Commands:\n\
	\x20 query <text>   Search and get LLM answer\n\
	\x20 q <text>       Alias for query\n\
	\x20 ingest <text>  Add text to the graph\n\
	\x20 health         Show graph statistics\n\
	\x20 list           Show first 50 entities\n\
	\x20 pulse          Trigger tick pulse\n\
	\x20 quit / exit    Exit REPL\n\
	\x20 ? / help       Show this help"
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::base::types::{Entity, Kern};

	#[test]
	fn help_text_lists_every_command() {
		let h = help_text();
		for cmd in ["query", "ingest", "health", "list", "pulse", "quit", "help"] {
			assert!(h.contains(cmd), "help text missing `{cmd}`");
		}
	}

	#[test]
	fn list_lines_caps_at_50_with_sentinel() {
		let mut g = GraphGnn::new();
		let mut k = Kern::new("kx", "");
		for i in 0..60 {
			let id = format!("e{i:04}");
			k.entities.insert(id.clone(), Entity { id, ..Default::default() });
		}
		g.kerns.insert("kx".into(), k);
		let lines = list_lines(&g);
		assert_eq!(lines.len(), 51, "50 entity lines + 1 truncation sentinel");
		assert_eq!(lines.last().unwrap(), "... (showing first 50)");
	}

	#[test]
	fn list_lines_empty_graph_says_no_entities() {
		let g = GraphGnn::new();
		assert_eq!(list_lines(&g), vec!["no entities".to_string()]);
	}
}
