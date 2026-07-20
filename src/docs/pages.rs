pub struct Page {
	pub slug: &'static str,
	pub section: &'static str,
	pub title: &'static str,
	pub body: &'static str,
}

pub static PAGES: &[Page] = &[
	Page {
		slug: "index",
		section: "",
		title: "kern",
		body: include_str!("../../docs/site/index.md"),
	},
	Page {
		slug: "why-kern",
		section: "Concepts",
		title: "Why kern exists",
		body: include_str!("../../docs/site/concepts/why-kern.md"),
	},
	Page {
		slug: "architecture",
		section: "Concepts",
		title: "Architecture — the working order, and why",
		body: include_str!("../../docs/site/concepts/architecture.md"),
	},
	Page {
		slug: "graph",
		section: "Concepts",
		title: "The graph: nodes, edges, claims",
		body: include_str!("../../docs/site/concepts/graph.md"),
	},
	Page {
		slug: "acceptance",
		section: "Concepts",
		title: "Acceptance & routing",
		body: include_str!("../../docs/site/concepts/acceptance.md"),
	},
	Page {
		slug: "time",
		section: "Concepts",
		title: "Time & contradiction",
		body: include_str!("../../docs/site/concepts/time.md"),
	},
	Page {
		slug: "retrieval",
		section: "Concepts",
		title: "The retrieval pipeline",
		body: include_str!("../../docs/site/concepts/retrieval.md"),
	},
	Page {
		slug: "heat-and-compaction",
		section: "Concepts",
		title: "Heat, decay & self-compaction",
		body: include_str!("../../docs/site/concepts/heat-and-compaction.md"),
	},
	Page {
		slug: "stigmergy",
		section: "Concepts",
		title: "Stigmergy GC & gravitons",
		body: include_str!("../../docs/site/concepts/stigmergy.md"),
	},
	Page {
		slug: "federation",
		section: "Concepts",
		title: "Federation",
		body: include_str!("../../docs/site/concepts/federation.md"),
	},
	Page {
		slug: "install-run",
		section: "How-to",
		title: "Install & run the daemon",
		body: include_str!("../../docs/site/howto/install-run.md"),
	},
	Page {
		slug: "memory-bank",
		section: "How-to",
		title: "The Memory Bank",
		body: include_str!("../../docs/site/howto/memory-bank.md"),
	},
	Page {
		slug: "intake-recall",
		section: "How-to",
		title: "Intake & recall in a session",
		body: include_str!("../../docs/site/howto/intake-recall.md"),
	},
	Page {
		slug: "seed",
		section: "How-to",
		title: "Seed the graph",
		body: include_str!("../../docs/site/howto/seed.md"),
	},
	Page {
		slug: "mcp",
		section: "How-to",
		title: "Wire up MCP",
		body: include_str!("../../docs/site/howto/mcp.md"),
	},
	Page {
		slug: "configure",
		section: "How-to",
		title: "Configure models",
		body: include_str!("../../docs/site/howto/configure.md"),
	},
];

#[cfg(test)]
mod tests {
	use super::PAGES;
	use std::collections::BTreeSet;

	fn on_disk() -> BTreeSet<String> {
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/site");
		let mut found = BTreeSet::new();
		let mut dirs = vec![root];
		while let Some(dir) = dirs.pop() {
			for entry in std::fs::read_dir(&dir).expect("read docs/site").flatten() {
				let path = entry.path();
				if path.is_dir() {
					dirs.push(path);
				} else if path.extension().is_some_and(|e| e == "md") {
					found.insert(path.file_stem().unwrap().to_string_lossy().into_owned());
				}
			}
		}
		found
	}

	#[test]
	fn every_page_on_disk_is_registered_and_vice_versa() {
		let disk = on_disk();
		let table: BTreeSet<String> = PAGES.iter().map(|p| p.slug.to_string()).collect();
		assert_eq!(
			disk, table,
			"docs/site/ and src/docs/pages.rs disagree — register the page (or drop the entry)"
		);
	}

	#[test]
	fn titles_match_the_first_heading() {
		for p in PAGES {
			let head = p
				.body
				.lines()
				.find_map(|l| l.strip_prefix("# "))
				.unwrap_or_else(|| panic!("{}: no `# ` heading", p.slug));
			assert_eq!(head.trim(), p.title, "{}: stale title", p.slug);
		}
	}
}
