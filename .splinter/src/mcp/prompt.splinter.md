# src/mcp/prompt.rs — commentary

- `QUERY_TOOL`/`INGEST_TOOL`: named constants rather than string literals buried in the `format!` so each tool name has a single definition; a rename in `tools.rs` fails the `research_prompt_names_are_real_tools` guard test instead of silently shipping a prompt that tells the model to call a tool that no longer exists.
To add a prompt: append to prompt_definitions() AND add a matching arm in handle_prompt_get; keep referenced tool names as consts (QUERY_TOOL/INGEST_TOOL) so the research_prompt_names_are_real_tools test catches renames in tools.rs.
