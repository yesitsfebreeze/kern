# src/commands/mcp_cmd.rs — commentary

Mode design: the proxy holds no graph, no tick worker, no ingest queue — every heavy bit lives in the daemon; it forwards via the typed `call_tool` escape hatch over kern.sock. Standalone is the legacy heavy path, kept so external MCP clients keep working when no daemon is up — it matches the pre-singleton behavior.

- `tools_list` (ProxyServer): forwards the daemon's LIVE tool list over kern_rpc so a client sees whatever the daemon actually exposes, not a static snapshot; the static catalogue (`tool_definitions`) is only the fallback when the list_tools RPC fails.
- `extra_capabilities` (ProxyServer): resources/prompts handlers currently fall through to method-not-found until they're proxied too — follow-up idea: route resources/* via a future KernRpc method.
- `tool_result_from_envelope`: kept pure so the envelope decoding is testable without a live kern.sock connection.
`kern mcp` is stdio MCP: proxy mode when a daemon owns kern.sock (forwards over kern_rpc), else a standalone fallback that loads a full local engine. attach_with_retry's short retry catches the "daemon up but slow to respond" race. In the standalone engine, question seeding and contradiction classification are both deferred to the tick (same wiring as the registry path): the worker carries no reason-LLM and stays embed-bound; the tick runs the reason/classify LLM and any bi-temporal supersedence. tool_result_from_envelope maps a kern_rpc call_tool reply to an MCP ToolResult, defaulting absent/mistyped fields to empty content / not-error.
