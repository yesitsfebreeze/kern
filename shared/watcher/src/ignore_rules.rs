use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Composite ignore matcher: per-root `.gitignore` + `.kernignore`.
///
/// Reuses the `ignore` crate (already a dep of `shared/search`) so we don't
/// duplicate gitignore semantics. Matchers are evaluated per-root: an event
/// is ignored if the path falls inside a root and the matcher for that root
/// reports a match.
pub struct IgnoreRules {
	per_root: Vec<RootRules>,
}

struct RootRules {
	root: PathBuf,
	gitignore: Option<Gitignore>,
	kernignore: Option<Gitignore>,
}

impl IgnoreRules {
	/// Build matchers by reading `<root>/.gitignore` and `<root>/.kernignore`
	/// for every root. Missing files are silently skipped (no rules).
	pub fn from_roots(roots: &[PathBuf]) -> Self {
		let per_root = roots
			.iter()
			.map(|r| {
				let root = r.clone();
				let gitignore = build(&root, ".gitignore");
				let kernignore = build(&root, ".kernignore");
				RootRules { root, gitignore, kernignore }
			})
			.collect();
		Self { per_root }
	}

	/// Empty matcher — nothing is ignored. Useful for tests.
	pub fn empty() -> Self {
		Self { per_root: Vec::new() }
	}

	/// Returns true if `path` should be skipped.
	///
	/// Every gitignore match below passes `is_dir = false` unconditionally. That
	/// is correct here because the watcher only ever feeds this function paths
	/// from notify filesystem events, which are file-shaped (a concrete path that
	/// changed), not directory listings. The `ignore` crate uses `is_dir` only to
	/// classify the matched path itself for trailing-slash patterns; a notify
	/// file event is never a directory, so `false` avoids a `stat` and still
	/// matches file patterns (`*.log`, `secret*`) as intended.
	pub fn is_ignored(&self, path: &Path) -> bool {
		// `.git/**` is always skipped — bursty internal churn we don't index.
		if path.components().any(|c| c.as_os_str() == ".git") {
			return true;
		}
		for rules in &self.per_root {
			let Ok(rel) = path.strip_prefix(&rules.root) else { continue };
			// `is_dir = false` — see the function doc; notify events are file-shaped.
			if let Some(g) = &rules.gitignore {
				if g.matched(rel, false).is_ignore() {
					return true;
				}
			}
			if let Some(g) = &rules.kernignore {
				if g.matched(rel, false).is_ignore() {
					return true;
				}
			}
		}
		false
	}
}

fn build(root: &Path, file: &str) -> Option<Gitignore> {
	let path = root.join(file);
	if !path.is_file() {
		return None;
	}
	let mut b = GitignoreBuilder::new(root);
	if b.add(&path).is_some() {
		// `add` returns `Some(error)` on failure; treat as no rules.
		return None;
	}
	b.build().ok()
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::tempdir;

	#[test]
	fn dot_git_paths_are_always_ignored() {
		// The `.git` guard fires regardless of configured rules (even empty()).
		let r = IgnoreRules::empty();
		assert!(r.is_ignored(Path::new("/repo/.git/HEAD")));
		assert!(r.is_ignored(Path::new("/repo/sub/.git/index")));
		assert!(!r.is_ignored(Path::new("/repo/src/main.rs")));
	}

	#[test]
	fn gitignore_patterns_match_relative_to_root() {
		let dir = tempdir().unwrap();
		std::fs::write(dir.path().join(".gitignore"), "*.log\ntarget\n").unwrap();
		let rules = IgnoreRules::from_roots(&[dir.path().to_path_buf()]);
		assert!(rules.is_ignored(&dir.path().join("server.log")), "*.log ignored");
		// A name pattern (`target`) matches that exact path. (The code matches the
		// event path itself via `Gitignore::matched`, which does not walk parents,
		// so a trailing-slash dir pattern would not catch nested files — `.git` is
		// the one recursive prune, handled separately above.)
		assert!(rules.is_ignored(&dir.path().join("target")), "named path ignored");
		assert!(!rules.is_ignored(&dir.path().join("src/main.rs")), "source kept");
	}

	#[test]
	fn kernignore_rules_are_honored_alongside_gitignore() {
		let dir = tempdir().unwrap();
		std::fs::write(dir.path().join(".kernignore"), "secret*\n").unwrap();
		let rules = IgnoreRules::from_roots(&[dir.path().to_path_buf()]);
		assert!(rules.is_ignored(&dir.path().join("secret.txt")), ".kernignore pattern matches");
		assert!(!rules.is_ignored(&dir.path().join("public.txt")));
	}

	#[test]
	fn paths_outside_any_root_are_not_ignored() {
		let dir = tempdir().unwrap();
		std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
		let rules = IgnoreRules::from_roots(&[dir.path().to_path_buf()]);
		// A .log path OUTSIDE the watched root fails strip_prefix -> not matched.
		assert!(!rules.is_ignored(Path::new("/elsewhere/server.log")));
	}

	#[test]
	fn empty_rules_ignore_nothing_except_dot_git() {
		let r = IgnoreRules::empty();
		assert!(!r.is_ignored(Path::new("/anything/file.log")));
		assert!(r.is_ignored(Path::new("/anything/.git/config")));
	}
}
