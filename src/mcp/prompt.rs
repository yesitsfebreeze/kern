use serde_json::value::RawValue;

use super::{err_resp, ok, Response, ERR_INVALID_REQ, ERR_NOT_FOUND};

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
								1. Use query(\"{topic}\") to see what's already known\n\
								2. Use query(\"{topic}\", answer=true) to get a synthesized answer\n\
								3. If knowledge is lacking, use ingest to add relevant text"
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
