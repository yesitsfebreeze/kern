// Kern runtime config. One TOML file per scope:
//   user:    <XDG_CONFIG>/kern/kern.toml
//   project: <cwd>/.kern/kern.toml
// Section-level merge: project sections replace user sections; missing
// fields fall through to Default.

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

pub use capture::CaptureConfig;
pub use embed::{DEFAULT_EMBED_MODEL, DEFAULT_EMBED_URL, EmbedConfig};
pub use gnn::GnnConfig;
pub use gossip::GossipConfig;
pub use graph::GraphConfig;
pub use ingest::IngestConfig;
pub use journal::JournalConfig;
pub use reason::ReasonConfig;
pub use retrieval::{ModeWeights, RetrievalConfig};
pub use serve::ServeConfig;
pub use tick::TickConfig;
pub use watcher::WatcherConfig;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::base::heat::HeatConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
	pub data_dir: String,
	pub log_level: String,
	pub embed: EmbedConfig,
	pub reason: ReasonConfig,
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
		let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
		Self {
			data_dir: cwd.join(".kern").to_string_lossy().into_owned(),
			log_level: "info".into(),
			embed: EmbedConfig::default(),
			reason: ReasonConfig::default(),
			serve: ServeConfig::default(),
			retrieval: RetrievalConfig::default(),
			ingest: IngestConfig::default(),
			gossip: GossipConfig::default(),
			tick: TickConfig::default(),
			heat: HeatConfig::defaults(),
			gnn: GnnConfig::default(),
			watcher: WatcherConfig::default(),
			capture: CaptureConfig::default(),
			graph: GraphConfig::default(),
			journal: JournalConfig::default(),
		}
	}
}

impl Config {
	pub fn load(cwd: &Path) -> Result<Self, config_io::Error> {
		// kern owns its own paths. User scope: <XDG_CONFIG>/kern/kern.toml
		// (absent is fine). Project scope: <cwd>/.kern/kern.toml.
		let user = dirs::config_dir()
			.unwrap_or_else(|| cwd.join(".kern"))
			.join("kern")
			.join("kern.toml");
		let project = cwd.join(".kern").join("kern.toml");
		config_io::load_layered(&user, &project)
	}

	pub fn validate(&self) -> Result<(), String> {
		if self.embed.url.is_empty() {
			return Err("embed.url is required".into());
		}
		if self.embed.model.is_empty() {
			return Err("embed.model is required".into());
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
}
