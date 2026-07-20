mod answer;
mod embed;
mod gnn;
mod gossip;
mod graph;
mod hub;
mod ingest;
mod intake;
pub mod io;
mod reason;
mod retrieval;
mod serve;
mod tick;
mod watcher;
mod wsl;

pub use answer::{AnswerConfig, DEFAULT_ANSWER_MODEL};
pub use embed::{EmbedConfig, DEFAULT_EMBED_MODEL, DEFAULT_EMBED_URL};
pub use gnn::GnnConfig;
pub use gossip::GossipConfig;
pub use graph::GraphConfig;
pub use hub::HubConfig;
pub use ingest::IngestConfig;
pub use intake::IntakeConfig;
pub use reason::ReasonConfig;
pub use retrieval::RetrievalConfig;
pub use serve::{mcp_token_path, ServeConfig};
pub use tick::TickConfig;
pub use watcher::WatcherConfig;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::base::heat::HeatConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
	pub data_dir: String,
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
	pub intake: IntakeConfig,
	pub graph: GraphConfig,
	pub hub: HubConfig,
}

impl Default for Config {
	fn default() -> Self {
		let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
		Self::default_in(&cwd)
	}
}

// Pin a relative `data_dir` to the load-time `cwd`: re-resolving against the
// live current_dir silently reads an empty graph from a wrong launch dir.
fn graviton_data_dir(data_dir: &str, cwd: &Path) -> String {
	let p = Path::new(data_dir);
	if p.is_absolute() {
		data_dir.to_string()
	} else {
		cwd.join(p).to_string_lossy().into_owned()
	}
}

impl Config {
	pub fn default_in(cwd: &Path) -> Self {
		Self {
			data_dir: cwd
				.join(".kern")
				.join("data")
				.to_string_lossy()
				.into_owned(),
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
			intake: IntakeConfig::default(),
			graph: GraphConfig::default(),
			hub: HubConfig::default(),
		}
	}

	pub fn load(cwd: &Path) -> Result<Self, io::Error> {
		let user = dirs::config_dir()
			.map(|d| d.join("kern").join("kern.toml"))
			.unwrap_or_else(|| cwd.join(".kern").join("kern.toml"));
		let project = cwd.join(".kern").join("kern.toml");
		let mut cfg: Self = io::load_layered(&user, &project)?;
		// serde's struct-level default pins data_dir to the *process* cwd. A
		// caller loading another root (hub merge, any cross-root tooling) must
		// get that root's store, never its own — re-pin when no config set it.
		if cfg.data_dir == Self::default().data_dir {
			cfg.data_dir = Self::default_in(cwd).data_dir;
		}
		cfg.data_dir = graviton_data_dir(&cfg.data_dir, cwd);
		cfg.redirect_loopback_to_wsl_host();
		Ok(cfg)
	}

	fn redirect_loopback_to_wsl_host(&mut self) {
		for (leg, url) in [
			("embed", &mut self.embed.url),
			("reason", &mut self.reason.url),
			("answer", &mut self.answer.url),
		] {
			if let Some(fixed) = wsl::resolve_loopback(url) {
				tracing::info!(
					target: "kern.config",
					leg, from = %url, to = %fixed,
					"WSL detected: loopback Ollama unreachable, using Windows host gateway"
				);
				*url = fixed;
			}
		}
	}

	// `.git` may be a FILE (worktree/submodule): test existence, not `is_dir()`.
	pub fn resolve_root(start: &Path) -> PathBuf {
		for anc in start.ancestors() {
			if anc.join(".git").exists() {
				return anc.to_path_buf();
			}
		}
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
		self.ingest.validate().map_err(|e| format!("ingest: {e}"))?;
		self.intake.validate().map_err(|e| format!("intake: {e}"))?;
		let retrieval = self.retrieval.validate();
		if !retrieval.is_empty() {
			return Err(format!("retrieval: {}", retrieval.join("; ")));
		}
		Ok(())
	}

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
	fn load_gravitons_relative_data_dir_to_cwd() {
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
	fn load_of_a_foreign_root_pins_data_dir_to_that_root() {
		// Regression: with no config file, serde's default pinned data_dir to the
		// *process* cwd — a cross-root load (hub merge) then read its own store.
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		std::fs::create_dir_all(root.join(".kern")).unwrap();

		let cfg = Config::load(&root).expect("load");
		assert_eq!(
			PathBuf::from(&cfg.data_dir),
			root.join(".kern").join("data"),
			"configless load must land in the passed root, not the process cwd"
		);
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
	fn resolve_root_gravitons_at_git_root_when_no_kern() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		std::fs::create_dir_all(root.join(".git")).unwrap();
		let deep = root.join("a").join("b");
		std::fs::create_dir_all(&deep).unwrap();

		assert_eq!(Config::resolve_root(&deep), root);
	}

	#[test]
	fn resolve_root_detects_git_as_a_file() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		std::fs::write(root.join(".git"), "gitdir: /elsewhere/.git/worktrees/x\n").unwrap();
		let deep = root.join("a");
		std::fs::create_dir_all(&deep).unwrap();

		assert_eq!(Config::resolve_root(&deep), root);
	}

	#[test]
	fn resolve_root_innermost_git_wins() {
		let dir = tempfile::tempdir().unwrap();
		let outer = dir.path().canonicalize().unwrap();
		std::fs::create_dir_all(outer.join(".git")).unwrap();
		let inner = outer.join("project");
		std::fs::create_dir_all(inner.join(".git")).unwrap();
		let deep = inner.join("src");
		std::fs::create_dir_all(&deep).unwrap();

		assert_eq!(Config::resolve_root(&deep), inner);
	}

	#[test]
	fn resolve_root_prefers_git_root_over_deeper_kern() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		std::fs::create_dir_all(root.join(".git")).unwrap();
		let sub = root.join("sub");
		std::fs::create_dir_all(sub.join(".kern")).unwrap();
		let deep = sub.join("deep");
		std::fs::create_dir_all(&deep).unwrap();

		assert_eq!(Config::resolve_root(&deep), root);
	}

	#[test]
	fn default_in_pins_data_dir_to_the_given_cwd_deterministically() {
		let cwd = Path::new("some_project_root");
		let cfg = Config::default_in(cwd);
		assert_eq!(
			cfg.data_dir,
			cwd.join(".kern").join("data").to_string_lossy()
		);
		assert_eq!(Config::default_in(cwd).data_dir, cfg.data_dir);
	}

	#[test]
	fn validate_requires_embed_and_surfaces_sub_config_invariants() {
		let cfg = Config::default_in(Path::new("x"));
		assert!(cfg.validate().is_ok(), "shipped defaults validate");

		let mut no_embed = Config::default_in(Path::new("x"));
		no_embed.embed.url = String::new();
		assert!(no_embed.validate().unwrap_err().contains("embed.url"));

		let mut bad_ingest = Config::default_in(Path::new("x"));
		bad_ingest.ingest.dedup_threshold = 2.0;
		let err = bad_ingest.validate().unwrap_err();
		assert!(
			err.contains("ingest"),
			"sub-config error is surfaced + tagged: {err}"
		);

		let mut bad_retr = Config::default_in(Path::new("x"));
		bad_retr.retrieval.seed_k = 0;
		let err = bad_retr.validate().unwrap_err();
		assert!(
			err.contains("retrieval"),
			"retrieval error surfaced + tagged: {err}"
		);
		assert!(err.contains("seed_k"), "the specific issue is named: {err}");
	}
}
