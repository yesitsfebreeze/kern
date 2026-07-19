pub fn tool_definitions() -> Vec<serde_json::Value> {
	let mut defs = super::tools_query::tool_schemas();
	defs.extend(super::tools_mutate::tool_schemas());
	defs.extend(super::tools_admin::tool_schemas());
	defs
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn definitions_are_well_formed_and_complete() {
		let defs = tool_definitions();
		let names: Vec<&str> = defs
			.iter()
			.map(|d| d["name"].as_str().expect("each tool has a string name"))
			.collect();

		let expected = [
			"query",
			"ingest",
			"link",
			"forget",
			"degrade",
			"health",
			"graviton",
			"descriptor",
			"pulse",
			"gc",
		];
		assert_eq!(names, expected, "tool set must match (order intentional)");

		for d in &defs {
			let name = d["name"].as_str().unwrap();
			assert!(!name.is_empty(), "tool name must not be empty");
			let schema = &d["inputSchema"];
			assert!(
				schema.is_object(),
				"{name}: inputSchema must be present and an object"
			);
			assert_eq!(
				schema["type"], "object",
				"{name}: inputSchema.type must be 'object'"
			);
		}
	}

	#[test]
	fn query_schema_requires_text_or_id() {
		let defs = tool_definitions();
		let query = defs
			.iter()
			.find(|d| d["name"] == "query")
			.expect("query tool present");
		let any_of = query["inputSchema"]["anyOf"]
			.as_array()
			.expect("query schema declares an anyOf branch");
		// Must mirror tool_query's runtime "either text or id is required" guard.
		let required: Vec<&str> = any_of
			.iter()
			.filter_map(|b| b["required"][0].as_str())
			.collect();
		assert!(
			required.contains(&"text"),
			"anyOf must allow `text`, got {required:?}"
		);
		assert!(
			required.contains(&"id"),
			"anyOf must allow `id`, got {required:?}"
		);
	}

	#[test]
	fn mutation_tools_declare_their_required_fields() {
		let defs = tool_definitions();
		// `required` must mirror each handler's runtime rejects, so clients fail fast.
		let want: &[(&str, &[&str])] = &[
			("ingest", &["text"]),
			("link", &["from", "to"]),
			("forget", &["id"]),
			("degrade", &["query_id"]),
			("descriptor", &["action", "name"]),
		];
		for (name, fields) in want {
			let tool = defs
				.iter()
				.find(|d| d["name"] == *name)
				.expect("tool present");
			let required: Vec<&str> = tool["inputSchema"]["required"]
				.as_array()
				.map(|a| a.iter().filter_map(|v| v.as_str()).collect())
				.unwrap_or_default();
			for f in *fields {
				assert!(
					required.contains(f),
					"{name} schema must require `{f}`, got {required:?}"
				);
			}
		}
	}
}
