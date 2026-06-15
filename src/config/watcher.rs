use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Configuration for the optional kern-side filesystem watcher.
///
/// Slice O — file changes flow into kern as `Document` entities through
/// `watcher::IngestSink`. The watcher is OFF by default; opt in via a
/// `[watcher]` section in `.kern/kern.toml`:
///
/// ```toml
/// [watcher]
/// enabled = true
/// roots = ["./src", "./docs"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherConfig {
	/// Master switch. False keeps the watcher dormant even if `roots` set.
	pub enabled: bool,
	/// Directory roots to watch (recursive). Empty defaults to cwd when
	/// `enabled = true` — resolved by [`WatcherConfig::effective_roots`].
	pub roots: Vec<String>,
}

impl WatcherConfig {
	/// The directories the watcher should actually watch: the configured `roots`,
	/// or — when `enabled` with no `roots` — a single fallback to `cwd`. Returns an
	/// empty vec when the watcher is disabled, so the caller can treat "nothing to
	/// watch" uniformly. `cwd` is injected (not read from the process) so the
	/// documented "empty defaults to cwd" rule lives in one place and is unit-testable.
	pub fn effective_roots(&self, cwd: &Path) -> Vec<PathBuf> {
		if !self.enabled {
			return Vec::new();
		}
		if self.roots.is_empty() {
			vec![cwd.to_path_buf()]
		} else {
			self.roots.iter().map(PathBuf::from).collect()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn effective_roots_falls_back_to_cwd_when_enabled_and_empty() {
		let cfg = WatcherConfig {
			enabled: true,
			roots: vec![],
		};
		assert_eq!(
			cfg.effective_roots(Path::new("/proj")),
			vec![PathBuf::from("/proj")]
		);
	}

	#[test]
	fn effective_roots_uses_configured_roots_when_present() {
		let cfg = WatcherConfig {
			enabled: true,
			roots: vec!["a".into(), "b".into()],
		};
		assert_eq!(
			cfg.effective_roots(Path::new("/proj")),
			vec![PathBuf::from("a"), PathBuf::from("b")],
			"configured roots win; cwd fallback is not applied"
		);
	}

	#[test]
	fn effective_roots_is_empty_when_disabled() {
		let cfg = WatcherConfig {
			enabled: false,
			roots: vec!["a".into()],
		};
		assert!(
			cfg.effective_roots(Path::new("/proj")).is_empty(),
			"a disabled watcher has nothing to watch even with roots set"
		);
	}
}
