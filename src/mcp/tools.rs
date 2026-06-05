pub fn tool_definitions() -> Vec<serde_json::Value> {
	vec![
		serde_json::json!({
			"name": "query",
			"description": "Search the knowledge graph. Returns scored thoughts and optionally an LLM answer.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"text":      {"type": "string", "description": "search query text"},
					"id":        {"type": "string", "description": "thought ID for direct lookup"},
					"k":         {"type": "integer", "description": "number of results (default 5)"},
					"mode":      {"type": "string", "enum": ["content", "reason", "hybrid"], "description": "retrieval mode (default hybrid)"},
					"answer":    {"type": "boolean", "description": "synthesize an LLM answer"},
					"sort":      {"type": "string", "enum": ["", "date", "access", "confidence"], "description": "sort key"},
					"ascending": {"type": "boolean", "description": "sort ascending (default false)"},
					"source":    {"type": "string", "description": "filter by source system"},
					"kind":      {"type": "string", "enum": ["", "normal", "fact", "document"], "description": "filter by thought kind"},
					"since":     {"type": "string", "description": "ISO8601 timestamp; only include thoughts at or after this time"},
					"before":    {"type": "string", "description": "ISO8601 timestamp; only include thoughts before this time"},
					"min_conf":  {"type": "number", "description": "minimum confidence 0.0-1.0"},
				},
			},
		}),
		serde_json::json!({
			"name": "ingest",
			"description": "Add text to the knowledge graph.",
			"inputSchema": {
				"type": "object",
				"required": ["text"],
				"properties": {
					"text":       {"type": "string", "description": "text to ingest"},
					"source":     {"type": "string", "description": "source system identifier"},
					"object_id":  {"type": "string", "description": "stable object identifier for update semantics"},
					"section":    {"type": "string", "description": "section within the object"},
					"author":     {"type": "string", "description": "author or origin of the content"},
					"title":      {"type": "string", "description": "human-readable title"},
					"url":        {"type": "string", "description": "URL reference"},
					"conf":       {"type": "number", "description": "confidence weight 0.0-1.0 (default 0.5)"},
					"descriptor": {"type": "string", "description": "Descriptor key for chunking context"},
					"sync":       {"type": "boolean", "description": "block until ingest completes (default false)"},
				},
			},
		}),
		serde_json::json!({
			"name": "link",
			"description": "Create a reason edge between two thoughts.",
			"inputSchema": {
				"type": "object",
				"required": ["from", "to"],
				"properties": {
					"from":   {"type": "string", "description": "source thought ID"},
					"to":     {"type": "string", "description": "target thought ID"},
					"reason": {"type": "string", "description": "reason text (LLM generates if empty)"},
				},
			},
		}),
		serde_json::json!({
			"name": "forget",
			"description": "Remove a thought and cascade-delete its edges. Facts are immune.",
			"inputSchema": {
				"type": "object",
				"required": ["id"],
				"properties": {
					"id": {"type": "string", "description": "thought ID to remove"},
				},
			},
		}),
		serde_json::json!({
			"name": "degrade",
			"description": "Decrease edge scores along the retrieval path for a query.",
			"inputSchema": {
				"type": "object",
				"required": ["query_id"],
				"properties": {
					"query_id": {"type": "string", "description": "thought ID at the end of a bad retrieval path"},
				},
			},
		}),
		serde_json::json!({
			"name": "health",
			"description": "Graph statistics: thought/edge counts, tick heat, unnamed count.",
			"inputSchema": {"type": "object", "properties": {}},
		}),
		serde_json::json!({
			"name": "purpose",
			"description": "Set or get the root Kern purpose.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"text": {"type": "string", "description": "new purpose text (omit to read current)"},
				},
			},
		}),
		serde_json::json!({
			"name": "descriptor",
			"description": "Add or remove a data-type descriptor.",
			"inputSchema": {
				"type": "object",
				"required": ["action", "name"],
				"properties": {
					"action":      {"type": "string", "enum": ["add", "rm"], "description": "add or remove"},
					"name":        {"type": "string", "description": "descriptor name"},
					"description": {"type": "string", "description": "markdown description (required for add)"},
				},
			},
		}),
		serde_json::json!({
			"name": "pulse",
			"description": "Trigger a pulse through the Kern tree, enqueuing cluster tasks for all kerns with thoughts.",
			"inputSchema": {
				"type": "object",
				"properties": {
					"strength": {"type": "number", "description": "pulse strength (default 1.0)"},
				},
			},
		}),
	]
}
