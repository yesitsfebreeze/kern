use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `[watcher]`: filesystem changes in as `Document` entities. OFF by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherConfig {
	/// Master switch. False keeps the watcher dormant even if `roots` set.
	pub enabled: bool,
	/// Recursive roots; empty defaults to cwd — see [`WatcherConfig::effective_roots`].
	pub roots: Vec<String>,
}

impl WatcherConfig {
	/// Configured `roots`, else `cwd` when enabled, else empty when disabled.
	/// `cwd` is injected rather than read from the process, for testability.
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
