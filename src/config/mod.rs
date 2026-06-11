// Kern runtime config. One TOML file per scope:
//   user:    <XDG_CONFIG>/kern/kern.toml
//   project: <cwd>/.kern/kern.toml
// Section-level merge: project sections replace user sections; missing
// fields fall through to Default.

mod answer;
mod capture;
mod embed;
mod gnn;
mod gossip;
mod graph;
mod ingest;
mod journal;
mod reason;
mod retrieval;
mod serve;
mod tick;
mod watcher;

pub use answer::{AnswerConfig, DEFAULT_ANSWER_MODEL};
pub use capture::CaptureConfig;
pub use embed::{DEFAULT_EMBED_MODEL, DEFAULT_EMBED_URL, EmbedConfig};
pub use gnn::GnnConfig;
pub use gossip::GossipConfig;
pub use graph::GraphConfig;
pub use ingest::IngestConfig;
pub use journal::{DEFAULT_MAX_TODAY_BYTES, DEFAULT_RETAIN_DAYS, JournalConfig};
pub use reason::{DEFAULT_REASON_MODEL, ReasonConfig};
pub use retrieval::{ModeWeights, RetrievalConfig};
pub use serve::ServeConfig;
pub use tick::TickConfig;
pub use watcher::WatcherConfig;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::base::heat::HeatConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
	pub data_dir: String,
	pub log_level: String,
	pub embed: EmbedConfig,
	pub reason: ReasonConfig,
	pub answer: AnswerConfig,
	pub serve: ServeConfig,
	pub retrieval: RetrievalConfig,
	pub ingest: IngestConfig,
	pub gossip: GossipConfig,
	pub tick: TickConfig,
	pub heat: HeatConfig,
	pub gnn: GnnConfig,
	pub watcher: WatcherConfig,
	pub capture: CaptureConfig,
	pub graph: GraphConfig,
	pub journal: JournalConfig,
}

impl Default for Config {
	fn default() -> Self {
		let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
		Self::default_in(&cwd)
	}
}

/// Resolve a possibly-relative `data_dir` to an absolute path under `cwd`, so
/// the instance pins the data location to the cwd it loaded from instead of
/// re-resolving a relative path (e.g. `.kern/data` from the project kern.toml)
/// against the process's live current_dir at every file op — which silently
/// reads/writes an empty graph when the daemon was launched from the wrong
/// directory. Absolute values pass through unchanged.
fn anchor_data_dir(data_dir: &str, cwd: &Path) -> String {
	let p = Path::new(data_dir);
	if p.is_absolute() {
		data_dir.to_string()
	} else {
		cwd.join(p).to_string_lossy().into_owned()
	}
}

impl Config {
	/// [`Default`], but with an explicit working directory instead of reading the
	/// process-wide `current_dir()`. Deterministic for tests and for callers that
	/// already know their root; `Config::default()` delegates here with the live
	/// cwd. Only `data_dir` depends on `cwd`; every other field is a fixed baseline.
	pub fn default_in(cwd: &Path) -> Self {
		Self {
			data_dir: cwd.join(".kern").to_string_lossy().into_owned(),
			log_level: "info".into(),
			embed: EmbedConfig::default(),
			reason: ReasonConfig::default(),
			answer: AnswerConfig::default(),
			serve: ServeConfig::default(),
			retrieval: RetrievalConfig::default(),
			ingest: IngestConfig::default(),
			gossip: GossipConfig::default(),
			tick: TickConfig::default(),
			heat: HeatConfig::default(),
			gnn: GnnConfig::default(),
			watcher: WatcherConfig::default(),
			capture: CaptureConfig::default(),
			graph: GraphConfig::default(),
			journal: JournalConfig::default(),
		}
	}

	pub fn load(cwd: &Path) -> Result<Self, config_io::Error> {
		// kern owns its own paths. User scope: <XDG_CONFIG>/kern/kern.toml
		// (absent is fine). Project scope: <cwd>/.kern/kern.toml.
		let user = dirs::config_dir()
			.unwrap_or_else(|| cwd.join(".kern"))
			.join("kern")
			.join("kern.toml");
		let project = cwd.join(".kern").join("kern.toml");
		let mut cfg: Self = config_io::load_layered(&user, &project)?;
		cfg.data_dir = anchor_data_dir(&cfg.data_dir, cwd);
		Ok(cfg)
	}

	/// Nearest ancestor of `start` (inclusive) that contains a `.kern`
	/// directory, else `start` itself. A kern instance launched from a
	/// subdirectory of a project still anchors to the project root, so it never
	/// boots an empty graph against a `.kern` that does not exist beside its
	/// accidental cwd.
	pub fn resolve_root(start: &Path) -> PathBuf {
		for anc in start.ancestors() {
			if anc.join(".kern").is_dir() {
				return anc.to_path_buf();
			}
		}
		start.to_path_buf()
	}

	pub fn validate(&self) -> Result<(), String> {
		if self.embed.url.is_empty() {
			return Err("embed.url is required".into());
		}
		if self.embed.model.is_empty() {
			return Err("embed.model is required".into());
		}
		// Section invariants: each sub-config validates its own ranges. Prefix the
		// section name so a bad value reports where it lives.
		self.ingest.validate().map_err(|e| format!("ingest: {e}"))?;
		self.capture.validate().map_err(|e| format!("capture: {e}"))?;
		Ok(())
	}

	/// Endpoint resolution precedence (applies to both URL and key):
	/// `reason_*` uses `[reason]` when set, else falls back to `[embed]`; `answer_*`
	/// uses `[answer]` when set, else falls back to the resolved `reason_*` (which in
	/// turn falls back to embed). So a single-Ollama deployment can fill in only
	/// `[embed]` and leave the reason/answer URLs empty — they all resolve to the
	/// embed endpoint, with each section still free to override just the model.
	pub fn reason_url(&self) -> &str {
		if self.reason.url.is_empty() {
			&self.embed.url
		} else {
			&self.reason.url
		}
	}

	pub fn reason_key(&self) -> &str {
		if self.reason.key.is_empty() {
			&self.embed.key
		} else {
			&self.reason.key
		}
	}

	/// Answer endpoint, falling back to the reason endpoint when `[answer]` omits
	/// a `url` — the common single-Ollama case where only the model differs.
	pub fn answer_url(&self) -> &str {
		if self.answer.url.is_empty() {
			self.reason_url()
		} else {
			&self.answer.url
		}
	}

	pub fn answer_key(&self) -> &str {
		if self.answer.key.is_empty() {
			self.reason_key()
		} else {
			&self.answer.key
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	#[test]
	fn load_anchors_relative_data_dir_to_cwd() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		let kern = root.join(".kern");
		std::fs::create_dir_all(&kern).unwrap();
		std::fs::write(kern.join("kern.toml"), "data_dir = \".kern/data\"\n").unwrap();

		let cfg = Config::load(&root).expect("load");

		let got = PathBuf::from(&cfg.data_dir);
		assert!(got.is_absolute(), "data_dir must be absolute, got {got:?}");
		assert_eq!(got, root.join(".kern").join("data"));
	}

	#[test]
	fn resolve_root_walks_up_to_nearest_kern_dir() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		std::fs::create_dir_all(root.join(".kern")).unwrap();
		let deep = root.join("a").join("b");
		std::fs::create_dir_all(&deep).unwrap();

		assert_eq!(Config::resolve_root(&deep), root);
	}

	#[test]
	fn resolve_root_returns_start_when_no_kern_ancestor() {
		let dir = tempfile::tempdir().unwrap();
		let start = dir.path().canonicalize().unwrap();
		assert_eq!(Config::resolve_root(&start), start);
	}

	#[test]
	fn default_in_pins_data_dir_to_the_given_cwd_deterministically() {
		let cwd = Path::new("some_project_root");
		let cfg = Config::default_in(cwd);
		assert_eq!(cfg.data_dir, cwd.join(".kern").to_string_lossy());
		assert_eq!(cfg.log_level, "info", "baseline fields are independent of cwd");
		// No process state read → two calls with the same cwd are identical.
		assert_eq!(Config::default_in(cwd).data_dir, cfg.data_dir);
	}

	#[test]
	fn validate_requires_embed_and_surfaces_sub_config_invariants() {
		let cfg = Config::default_in(Path::new("x"));
		assert!(cfg.validate().is_ok(), "shipped defaults validate");

		let mut no_embed = Config::default_in(Path::new("x"));
		no_embed.embed.url = String::new();
		assert!(no_embed.validate().unwrap_err().contains("embed.url"));

		// A bad ingest knob now propagates through the top-level validate, tagged
		// with its section.
		let mut bad_ingest = Config::default_in(Path::new("x"));
		bad_ingest.ingest.rephrase_lower = 0.9;
		bad_ingest.ingest.rephrase_upper = 0.8;
		let err = bad_ingest.validate().unwrap_err();
		assert!(err.contains("ingest"), "sub-config error is surfaced + tagged: {err}");
	}
}
