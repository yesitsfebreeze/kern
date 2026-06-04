use serde::{Deserialize, Serialize};

/// Configuration for the optional kern-side filesystem watcher.
///
/// Slice O — file changes flow into kern as `Document` entities through
/// `watcher::IngestSink`. The watcher is OFF by default; opt in via a
/// `[watcher]` section in `.relay/kern.toml`:
///
/// ```toml
/// [watcher]
/// enabled = true
/// roots = ["./src", "./docs"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WatcherConfig {
	/// Master switch. False keeps the watcher dormant even if `roots` set.
	pub enabled: bool,
	/// Directory roots to watch (recursive). Empty defaults to cwd when
	/// `enabled = true`.
	pub roots: Vec<String>,
}

