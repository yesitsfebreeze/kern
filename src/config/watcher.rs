use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherConfig {
	pub enabled: bool,
	pub roots: Vec<String>,
}

impl WatcherConfig {
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
