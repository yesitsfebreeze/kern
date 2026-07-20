mod menu;
mod pages;

use std::io::IsTerminal;

use pages::PAGES;

pub fn run(page: Option<&str>, list: bool) -> anyhow::Result<()> {
	if list {
		for p in PAGES {
			println!("{}\t{}\t{}", p.slug, p.section, p.title);
		}
		return Ok(());
	}

	if let Some(slug) = page {
		print!("{}", PAGES[resolve(slug)?].body);
		return Ok(());
	}

	// An LLM or a pipe must never meet a prompt it cannot answer.
	if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
		print!("{}", PAGES[0].body);
		println!("\n## Pages\n");
		for p in PAGES {
			println!("- `kern docs {}` — {}", p.slug, p.title);
		}
		return Ok(());
	}

	let stdin = std::io::stdin();
	if let Some(i) = menu::pick(&mut stdin.lock(), &mut std::io::stdout())? {
		println!();
		print!("{}", PAGES[i].body);
	}
	Ok(())
}

fn resolve(slug: &str) -> anyhow::Result<usize> {
	let want = slug.trim().trim_end_matches(".md").to_lowercase();
	if let Some(i) = PAGES.iter().position(|p| p.slug == want) {
		return Ok(i);
	}
	let near: Vec<&str> = PAGES
		.iter()
		.filter(|p| {
			p.slug.contains(&want) || want.contains(p.slug) || p.title.to_lowercase().contains(&want)
		})
		.map(|p| p.slug)
		.collect();
	let suggestions = if near.is_empty() {
		PAGES.iter().map(|p| p.slug).collect::<Vec<_>>()
	} else {
		near
	};
	anyhow::bail!(
		"no such page '{slug}'. did you mean: {}?",
		suggestions.join(", ")
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn known_slugs_resolve_and_unknown_ones_fail_with_suggestions() {
		assert!(resolve("retrieval").is_ok());
		assert!(
			resolve("retrieval.md").is_ok(),
			"the .md suffix is forgiven"
		);
		let err = resolve("retriev").unwrap_err().to_string();
		assert!(err.contains("retrieval"), "{err}");
		assert!(resolve("zzzz").is_err());
	}
}
