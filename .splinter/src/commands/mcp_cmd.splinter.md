# src/commands/mcp_cmd.rs — commentary

Mode design: the proxy holds no graph, no tick worker, no ingest queue — every heavy bit lives in the daemon; it forwards via the typed `call_tool` escape hatch over kern.sock. Standalone is the legacy heavy path, kept so external MCP clients (Claude Desktop, etc.) keep working when no daemon is up — it matches the pre-singleton behavior.

- `tools_list` (ProxyServer): forwards the daemon's LIVE tool list over kern_rpc so a client sees whatever the daemon actually exposes, not a static snapshot; the static catalogue (`tool_definitions`) is only the fallback when the list_tools RPC fails.
- `extra_capabilities` (ProxyServer): resources/prompts handlers currently fall through to method-not-found until they're proxied too — follow-up idea: route resources/* via a future KernRpc method.
- `tool_result_from_envelope`: kept pure so the envelope decoding is testable without a live kern.sock connection.
