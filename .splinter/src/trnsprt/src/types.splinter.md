# src/trnsprt/src/types.rs — commentary

Second-pass migration:

- `ToolSchema::input_schema`: the 6-line rationale for the opaque `Value` is here now, 2 lines left inline. Full reasoning — it is NOT a typed Rust struct on purpose: MCP `inputSchema` is forwarded verbatim between client and server, and every tool defines its own argument shape, so binding it to one Rust type here would force a lossy translation. Late-binding by design: the schema is validated by the consuming model/host, not by this transport. `None` means the tool takes no arguments (that part stayed inline — it's a wire contract).
- `ToolResult`/`ToolSchema` derive `PartialEq` but not `Eq`: they hold `serde_json::Value`, whose number variant is an `f64`, so `Value: !Eq`. Compressed to one line inline — `==` is all the test assertions need.
Design notes (moved from source comments during comment sweep):
- ToolSchema.input_schema is an opaque Value: MCP inputSchema is forwarded verbatim and validated by the consuming host, not here. None means the tool takes no arguments.
- ToolResult derives only PartialEq (not Eq): serde_json::Value's number variant is f64, so Value: !Eq.
