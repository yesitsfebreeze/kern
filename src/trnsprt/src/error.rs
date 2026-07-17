use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
	#[error("mcp transport i/o: {0}")]
	Io(#[from] std::io::Error),
	#[error("mcp protocol: {0}")]
	Protocol(String),
	#[error("mcp json: {0}")]
	Json(#[from] serde_json::Error),
	#[error("mcp rpc error {code}: {message}")]
	Rpc { code: i64, message: String },
	#[error("unknown mcp server: {0}")]
	UnknownServer(String),
	#[error("mcp server already registered: {0}")]
	DuplicateServer(String),
	#[error("mcp child process not running")]
	NotRunning,
}

impl McpError {
	pub fn is_transient(&self) -> bool {
		matches!(self, McpError::Io(_) | McpError::NotRunning)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_transient_is_true_only_for_connection_level_faults() {
		use std::io::{Error as IoError, ErrorKind};
		assert!(McpError::Io(IoError::new(ErrorKind::BrokenPipe, "reset")).is_transient());
		assert!(McpError::NotRunning.is_transient());

		assert!(!McpError::Protocol("missing tools".into()).is_transient());
		assert!(!McpError::Rpc {
			code: -32601,
			message: "no method".into()
		}
		.is_transient());
		assert!(!McpError::UnknownServer("s".into()).is_transient());
		assert!(!McpError::DuplicateServer("s".into()).is_transient());

		let json_err = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
		assert!(
			!McpError::Json(json_err).is_transient(),
			"a parse failure is deterministic"
		);
	}
}
