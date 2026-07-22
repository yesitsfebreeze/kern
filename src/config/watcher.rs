use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatcherConfig {
	pub enabled: bool,
	pub roots: Vec<String>,
	// The TTL stamped on every document this watcher sinks — the watched roots
	// are one source, so their retention is one policy. Same reason as
	// `intake.retention_secs` for living here and not in the preset-owned
	// `[ingest]`: a user's `kern.toml` may set nothing in that table but
	// `review_policy`, and a tuning key there refuses to load.
	// 0 = no TTL. Derived `Default` gives 0, which is the shipped behaviour.
	pub retention_secs: u64,
}

impl WatcherConfig {
	pub fn validate(&self) -> Result<(), String> {
		crate::ingest::valid_until_from_retention(self.retention_secs)?;
		Ok(())
	}

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
			..Default::default()
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
			..Default::default()
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
			..Default::default()
		};
		assert!(
			cfg.effective_roots(Path::new("/proj")).is_empty(),
			"a disabled watcher has nothing to watch even with roots set"
		);
	}
}
