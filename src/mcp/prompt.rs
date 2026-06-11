use serde_json::value::RawValue;

use super::{err_resp, ok, Response, ERR_INVALID_REQ, ERR_NOT_FOUND};

// Tool names the `research` prompt body steers the model toward. Kept as named
// constants — not inline string literals buried in the format! — so they have a
// single definition, and guarded by the `research_prompt_names_are_real_tools`
// test: a rename in `tools.rs` fails that test instead of silently shipping a
// prompt that tells the model to call a tool that no longer exists.
const QUERY_TOOL: &str = "query";
const INGEST_TOOL: &str = "ingest";

/// MCP prompt catalogue advertised to clients.
///
/// To add a prompt: append its definition here AND add a matching arm in
/// [`handle_prompt_get`] keyed on the same `name`. Keep any tool names the
/// prompt body references as `const` (see [`QUERY_TOOL`]) and add them to the
/// guard test so the two never drift.
pub fn prompt_definitions() -> Vec<serde_json::Value> {
	vec![serde_json::json!({
		"name": "research",
		"description": "Use the kern knowledge graph to research a topic",
		"arguments": [{
			"name": "topic",
			"description": "The topic to research",
			"required": true,
		}],
	})]
}

pub(crate) fn handle_prompt_get(id: Option<Box<RawValue>>, params: Option<Box<RawValue>>) -> Response {
	#[derive(serde::Deserialize)]
	struct Params {
		name: String,
		#[serde(default)]
		arguments: std::collections::HashMap<String, String>,
	}

	let params: Params = match params
		.as_deref()
		.map(|r| serde_json::from_str(r.get()))
		.transpose()
	{
		Ok(Some(p)) => p,
		_ => return err_resp(id, ERR_INVALID_REQ, "invalid params"),
	};

	match params.name.as_str() {
		"research" => {
			let topic = params.arguments.get("topic").cloned().unwrap_or_default();
			if topic.is_empty() {
				return err_resp(id, ERR_INVALID_REQ, "topic argument required");
			}
			ok(
				id,
				serde_json::json!({
					"messages": [{
						"role": "user",
						"content": {
							"type": "text",
							"text": format!(
								"Use the kern knowledge graph to answer questions about: {topic}\n\n\
								1. Use {q}(\"{topic}\") to see what's already known\n\
								2. Use {q}(\"{topic}\", answer=true) to get a synthesized answer\n\
								3. If knowledge is lacking, use {ing} to add relevant text",
								q = QUERY_TOOL,
								ing = INGEST_TOOL,
							),
						},
					}],
				}),
			)
		}
		_ => err_resp(
			id,
			ERR_NOT_FOUND,
			&format!("unknown prompt: {}", params.name),
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn raw(v: serde_json::Value) -> Box<RawValue> {
		serde_json::value::to_raw_value(&v).unwrap()
	}

	/// Pull the prompt message text out of a successful response.
	fn message_text(resp: &Response) -> String {
		resp.result.as_ref().expect("result present")["messages"][0]["content"]["text"]
			.as_str()
			.expect("text content")
			.to_string()
	}

	#[test]
	fn happy_path_embeds_topic_in_message() {
		let params = raw(serde_json::json!({
			"name": "research",
			"arguments": { "topic": "borrow checker" },
		}));
		let resp = handle_prompt_get(None, Some(params));
		assert!(resp.error.is_none(), "no error on valid request");
		assert!(message_text(&resp).contains("borrow checker"));
	}

	#[test]
	fn missing_params_is_invalid_request() {
		let resp = handle_prompt_get(None, None);
		let err = resp.error.as_ref().expect("error present");
		assert_eq!(err.code, ERR_INVALID_REQ);
	}

	#[test]
	fn unknown_prompt_name_is_not_found() {
		let params = raw(serde_json::json!({ "name": "no_such_prompt" }));
		let resp = handle_prompt_get(None, Some(params));
		let err = resp.error.as_ref().expect("error present");
		assert_eq!(err.code, ERR_NOT_FOUND);
	}

	#[test]
	fn empty_topic_is_rejected() {
		let params = raw(serde_json::json!({
			"name": "research",
			"arguments": { "topic": "" },
		}));
		let resp = handle_prompt_get(None, Some(params));
		let err = resp.error.as_ref().expect("error present");
		assert_eq!(err.code, ERR_INVALID_REQ);
	}

	#[test]
	fn happy_path_steers_to_query_and_ingest_tools() {
		let params = raw(serde_json::json!({
			"name": "research",
			"arguments": { "topic": "graphs" },
		}));
		let text = message_text(&handle_prompt_get(None, Some(params)));
		assert!(text.contains(QUERY_TOOL), "prompt should reference the query tool");
		assert!(text.contains(INGEST_TOOL), "prompt should reference the ingest tool");
	}

	/// Guard against silent drift: the tool names the `research` prompt tells the
	/// model to call must actually exist in `tool_definitions()`. If a tool is
	/// renamed in `tools.rs` without updating the constants here, this fails.
	#[test]
	fn research_prompt_names_are_real_tools() {
		let names: Vec<String> = crate::mcp::tools::tool_definitions()
			.iter()
			.filter_map(|d| d.get("name").and_then(|n| n.as_str()).map(String::from))
			.collect();
		for tool in [QUERY_TOOL, INGEST_TOOL] {
			assert!(
				names.contains(&tool.to_string()),
				"research prompt references tool `{tool}` absent from tool_definitions()"
			);
		}
	}
}
