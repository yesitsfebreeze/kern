use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub struct IgnoreRules {
	per_root: Vec<RootRules>,
}

struct RootRules {
	root: PathBuf,
	gitignore: Option<Gitignore>,
	kernignore: Option<Gitignore>,
}

impl IgnoreRules {
	pub fn from_roots(roots: &[PathBuf]) -> Self {
		let per_root = roots
			.iter()
			.map(|r| {
				let root = r.clone();
				let gitignore = build(&root, ".gitignore");
				let kernignore = build(&root, ".kernignore");
				RootRules {
					root,
					gitignore,
					kernignore,
				}
			})
			.collect();
		Self { per_root }
	}

	pub fn empty() -> Self {
		Self {
			per_root: Vec::new(),
		}
	}

	// `matched(rel, false)`: notify event paths are never directory listings, so `is_dir` is always false.
	pub fn is_ignored(&self, path: &Path) -> bool {
		// `.git` always skipped — bursty internal churn, never removed even if unignored.
		if path.components().any(|c| c.as_os_str() == ".git") {
			return true;
		}
		for rules in &self.per_root {
			let Ok(rel) = path.strip_prefix(&rules.root) else {
				continue;
			};
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
		// `add` returns `Some(error)` on failure (not success); treat as no rules.
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
		assert!(
			rules.is_ignored(&dir.path().join("server.log")),
			"*.log ignored"
		);
		assert!(
			rules.is_ignored(&dir.path().join("target")),
			"named path ignored"
		);
		assert!(
			!rules.is_ignored(&dir.path().join("src/main.rs")),
			"source kept"
		);
	}

	#[test]
	fn kernignore_rules_are_honored_alongside_gitignore() {
		let dir = tempdir().unwrap();
		std::fs::write(dir.path().join(".kernignore"), "secret*\n").unwrap();
		let rules = IgnoreRules::from_roots(&[dir.path().to_path_buf()]);
		assert!(
			rules.is_ignored(&dir.path().join("secret.txt")),
			".kernignore pattern matches"
		);
		assert!(!rules.is_ignored(&dir.path().join("public.txt")));
	}

	#[test]
	fn paths_outside_any_root_are_not_ignored() {
		let dir = tempdir().unwrap();
		std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
		let rules = IgnoreRules::from_roots(&[dir.path().to_path_buf()]);
		assert!(!rules.is_ignored(Path::new("/elsewhere/server.log")));
	}

	#[test]
	fn empty_rules_ignore_nothing_except_dot_git() {
		let r = IgnoreRules::empty();
		assert!(!r.is_ignored(Path::new("/anything/file.log")));
		assert!(r.is_ignored(Path::new("/anything/.git/config")));
	}
}
