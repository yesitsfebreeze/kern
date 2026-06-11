use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
	/// Transport-level I/O failure on the underlying pipe/socket (broken pipe,
	/// EOF mid-frame, connection reset). Connection-level and retryable — see
	/// [`is_transient`](Self::is_transient).
	#[error("mcp transport i/o: {0}")]
	Io(#[from] std::io::Error),
	/// The peer spoke malformed MCP: a missing required field, a frame containing
	/// an embedded newline, non-UTF-8 bytes, etc. A wire-format violation — distinct
	/// from [`Rpc`](Self::Rpc), which is a *well-formed* error response.
	#[error("mcp protocol: {0}")]
	Protocol(String),
	/// JSON (de)serialisation of a frame body failed.
	#[error("mcp json: {0}")]
	Json(#[from] serde_json::Error),
	/// A well-formed JSON-RPC error response from the server — the call reached the
	/// peer and it replied `{ error: { code, message } }`. Application-level, NOT a
	/// transport fault; `code` follows JSON-RPC conventions (e.g. `-32601`
	/// method-not-found, `-32602` invalid-params).
	#[error("mcp rpc error {code}: {message}")]
	Rpc { code: i64, message: String },
	/// A call targeted a [`ServerId`](crate::ServerId) not present in the registry.
	#[error("unknown mcp server: {0}")]
	UnknownServer(String),
	/// Tried to register a [`ServerId`](crate::ServerId) that is already registered
	/// (ids must be unique within a registry).
	#[error("mcp server already registered: {0}")]
	DuplicateServer(String),
	/// The child MCP server process is not running (never started, or has exited).
	/// Connection-level — a supervisor may respawn it and retry.
	#[error("mcp child process not running")]
	NotRunning,
}

impl McpError {
	/// Whether retrying the operation could plausibly succeed. `true` only for
	/// connection-level faults — [`Io`](Self::Io) (a pipe hiccup / reset) and
	/// [`NotRunning`](Self::NotRunning) (the child can be respawned and the call
	/// re-sent). [`Protocol`](Self::Protocol), [`Json`](Self::Json),
	/// [`Rpc`](Self::Rpc), [`UnknownServer`](Self::UnknownServer), and
	/// [`DuplicateServer`](Self::DuplicateServer) are deterministic given the same
	/// input, so a retry just reproduces them. Lets callers gate retries on one
	/// predicate instead of matching every arm.
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
		assert!(!McpError::Rpc { code: -32601, message: "no method".into() }.is_transient());
		assert!(!McpError::UnknownServer("s".into()).is_transient());
		assert!(!McpError::DuplicateServer("s".into()).is_transient());

		let json_err = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
		assert!(!McpError::Json(json_err).is_transient(), "a parse failure is deterministic");
	}
}
