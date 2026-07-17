//! Tool schema, tool result, and server-id value types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
	pub name: String,
	#[serde(default)]
	pub description: Option<String>,
	/// Opaque `Value`: MCP `inputSchema` is forwarded verbatim and validated by the
	/// consuming host, not here. `None` means the tool takes no arguments.
	#[serde(default, rename = "inputSchema")]
	pub input_schema: Option<Value>,
}

// Only `PartialEq`: `serde_json::Value`'s number variant is `f64`, so `Value: !Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
	#[serde(default)]
	pub content: Vec<Value>,
	#[serde(default, rename = "isError")]
	pub is_error: bool,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		rename = "structuredContent"
	)]
	pub structured_content: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerId(pub String);

impl ServerId {
	pub fn new<S: Into<String>>(id: S) -> Self {
		Self(id.into())
	}
}

impl std::fmt::Display for ServerId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn tool_schema_and_result_support_equality() {
		let a = ToolSchema {
			name: "add".into(),
			description: Some("a+b".into()),
			input_schema: Some(json!({ "type": "object" })),
		};
		assert_eq!(a, a.clone(), "PartialEq compares whole schemas in one ==");
		let b = ToolSchema {
			name: "sub".into(),
			..a.clone()
		};
		assert_ne!(a, b);

		let r = ToolResult {
			content: vec![json!({ "type": "text", "text": "ok" })],
			is_error: false,
			structured_content: None,
		};
		assert_eq!(r, r.clone());
	}

	#[test]
	fn server_id_displays_as_its_inner_string() {
		assert_eq!(ServerId::new("math").to_string(), "math");
		assert_eq!(format!("[{}]", ServerId::new("srv1")), "[srv1]");
	}
}
