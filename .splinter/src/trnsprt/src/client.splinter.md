# src/trnsprt/src/client.rs — commentary

Second-pass migration:

- `Client` doc went from 10 lines to 2 (purpose + the one trap: at most one in-flight request). Moved here:
  - Each request is one `{jsonrpc, id, method, params}` object serialized to a single line and flushed; the matching reply is the line whose `id` equals the request's.
  - Fully SYNCHRONOUS — `request` blocks reading frames off the transport until it sees its id. A `Client` is therefore NOT meant for concurrent calls from multiple threads. This is the trap that stayed inline.
  - Notifications carry no `id` and expect no reply.
  - `rx_buf` retains bytes read past a frame boundary for the next `recv`.
  - `send` rejects frames containing an embedded newline (the wire is newline-delimited).
- `MAX_UNMATCHED_FRAMES` (1024): compressed to the constraint. In normal use only a handful of notifications are skipped before the matching reply arrives; the cap exists so a peer flooding unrelated frames, or a wire desync, can't spin the read loop forever.
Design notes (moved from source comments during comment sweep):
- Client is a blocking MCP client over a newline-delimited JSON-RPC wire. At most one in-flight request — Client::request blocks until its id comes back.
- HAZARD (kept tightened in source): MAX_UNMATCHED_FRAMES bounds Client::request's read loop so a peer flooding unrelated frames (or a wire desync) can't spin forever.
