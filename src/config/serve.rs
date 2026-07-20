use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeConfig {
	// Empty = read (or mint) the token file instead; see `resolve_mcp_token`.
	pub mcp_token: String,
}

pub fn mcp_token_path(data_dir: &Path) -> PathBuf {
	data_dir.join("mcp-token")
}

fn mint_token() -> String {
	use rand::RngExt;
	let mut rng = rand::rng();
	format!(
		"{:016x}{:016x}{:016x}{:016x}",
		rng.random::<u64>(),
		rng.random::<u64>(),
		rng.random::<u64>(),
		rng.random::<u64>()
	)
}

// Written 0600 before any content, so the secret is never briefly world-readable.
#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
	use std::os::unix::fs::OpenOptionsExt;
	std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.mode(0o600)
		.open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
	std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)
}

impl ServeConfig {
	/// The token the HTTP/SSE surface must demand. An explicit `mcp_token` wins;
	/// otherwise the per-graph token file is read, minting it on first use so a
	/// local user never has to configure anything.
	pub fn resolve_mcp_token(&self, data_dir: &Path) -> std::io::Result<String> {
		if !self.mcp_token.is_empty() {
			return Ok(self.mcp_token.clone());
		}
		let path = mcp_token_path(data_dir);
		match std::fs::read_to_string(&path) {
			Ok(t) if !t.trim().is_empty() => return Ok(t.trim().to_string()),
			Ok(_) => {
				let _ = std::fs::remove_file(&path);
			}
			Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
			Err(_) => {}
		}
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let token = mint_token();
		match create_private(&path) {
			Ok(mut f) => {
				use std::io::Write;
				f.write_all(token.as_bytes())?;
				Ok(token)
			}
			// Lost the create race to a sibling process: its token is the real one.
			Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
				Ok(std::fs::read_to_string(&path)?.trim().to_string())
			}
			Err(e) => Err(e),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_explicit_token_wins_over_the_file() {
		let dir = tempfile::tempdir().unwrap();
		let cfg = ServeConfig {
			mcp_token: "configured".into(),
		};
		assert_eq!(cfg.resolve_mcp_token(dir.path()).unwrap(), "configured");
		assert!(
			!mcp_token_path(dir.path()).exists(),
			"an explicit token mints no file"
		);
	}

	#[test]
	fn a_token_is_minted_once_and_then_reused() {
		let dir = tempfile::tempdir().unwrap();
		let cfg = ServeConfig::default();
		let first = cfg.resolve_mcp_token(dir.path()).unwrap();
		assert_eq!(first.len(), 64, "256 bits of hex");
		assert_eq!(
			cfg.resolve_mcp_token(dir.path()).unwrap(),
			first,
			"a second resolve reuses the minted token"
		);
	}

	#[cfg(unix)]
	#[test]
	fn the_minted_token_file_is_owner_only() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		ServeConfig::default()
			.resolve_mcp_token(dir.path())
			.unwrap();
		let mode = std::fs::metadata(mcp_token_path(dir.path()))
			.unwrap()
			.permissions()
			.mode()
			& 0o777;
		assert_eq!(
			mode, 0o600,
			"the token must not be world-readable: {mode:o}"
		);
	}
}
