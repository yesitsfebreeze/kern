use std::io::{BufRead, Write};

use super::pages::PAGES;

/// A numbered topic list. No raw mode, no alternate screen: the page it picks is
/// printed into normal scrollback, so the terminal's own scroll and search read it.
/// `None` means the reader backed out.
pub fn pick(input: &mut impl BufRead, out: &mut impl Write) -> anyhow::Result<Option<usize>> {
	writeln!(out, "kern docs\n")?;
	let mut section = "";
	for (i, p) in PAGES.iter().enumerate() {
		if p.section != section {
			section = p.section;
			if !section.is_empty() {
				writeln!(out, "  {}", section.to_uppercase())?;
			}
		}
		writeln!(out, "  {:>2}. {:<22}{}", i + 1, p.slug, p.title)?;
	}
	write!(out, "\npick a number or name (q to quit): ")?;
	out.flush()?;

	let mut line = String::new();
	if input.read_line(&mut line)? == 0 {
		return Ok(None);
	}
	let answer = line.trim();
	if answer.is_empty() || answer.eq_ignore_ascii_case("q") {
		return Ok(None);
	}
	if let Ok(n) = answer.parse::<usize>() {
		return Ok((1..=PAGES.len()).contains(&n).then(|| n - 1));
	}
	super::resolve(answer).map(Some)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pick_from(input: &str) -> anyhow::Result<Option<usize>> {
		pick(&mut input.as_bytes(), &mut Vec::new())
	}

	fn slug(i: usize) -> &'static str {
		PAGES[i].slug
	}

	#[test]
	fn a_number_picks_that_row_and_out_of_range_backs_out() {
		assert_eq!(pick_from("1\n").unwrap(), Some(0));
		assert_eq!(
			pick_from(&format!("{}\n", PAGES.len())).unwrap(),
			Some(PAGES.len() - 1)
		);
		assert_eq!(pick_from("0\n").unwrap(), None, "numbering starts at 1");
		assert_eq!(pick_from(&format!("{}\n", PAGES.len() + 1)).unwrap(), None);
	}

	#[test]
	fn a_name_picks_the_page_and_tolerates_the_md_suffix() {
		let i = pick_from("retrieval\n").unwrap().unwrap();
		assert_eq!(slug(i), "retrieval");
		assert_eq!(pick_from("retrieval.md\n").unwrap(), Some(i));
	}

	#[test]
	fn quitting_eof_and_an_empty_line_all_back_out() {
		for input in ["q\n", "Q\n", "\n", ""] {
			assert_eq!(pick_from(input).unwrap(), None, "{input:?}");
		}
	}

	#[test]
	fn an_unknown_name_reports_the_near_misses() {
		let err = pick_from("retriev\n").unwrap_err().to_string();
		assert!(err.contains("retrieval"), "{err}");
	}

	#[test]
	fn the_listing_names_every_shipped_page() {
		let mut buf = Vec::new();
		pick(&mut "q\n".as_bytes(), &mut buf).unwrap();
		let out = String::from_utf8(buf).unwrap();
		for p in PAGES {
			assert!(out.contains(p.slug), "{} missing from the menu", p.slug);
		}
	}
}
