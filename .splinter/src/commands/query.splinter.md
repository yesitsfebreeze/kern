# src/commands/query.rs — commentary

- `QueryParams`: exists so `cmd_query` takes one struct instead of an eight-positional signature; named `QueryParams` (not `QueryArgs`) to avoid colliding with the deserialize-side `QueryArgs` in `mcp/tools_query`.
- `cmd_query` answer path: single-shot (`stream: false`) is one round-trip collected into the printed answer; the streamed tokens arrive through the same interface the `/ask` UI consumes incrementally — see `Client::answer`.
