# src/main.rs — commentary

Serving-daemon design: kern is a pure serving daemon — the bare invocation, `--daemon`, and the `daemon` subcommand all boot the server and bind the channels (MCP over stdio/HTTP-SSE + kern_rpc); a subcommand runs one one-shot tool against the graph. There is deliberately no interactive surface — every tool is reached over a channel (CLI subcommand or MCP), never a REPL or TUI.

- `worker_thread_count`: kept pure so the floor logic is unit-tested.
- config validation: deliberately non-fatal (consistent with the tolerant `Config::load` fallback) — a misconfigured value degrades behaviour with a loud warn instead of refusing to boot; it used to be silently invisible.
- cwd re-pin log: exists because operators inspecting where the daemon anchored its data_dir/intake need to see the re-pin — a silent cwd change is hard to diagnose.

Second-pass migration: the cwd re-pin comment kept only the trap (a subdir launch would boot an empty graph while still serving queries). The moved detail — pinning to the nearest ancestor with `.kern` is what makes the endpoint tag, `data_dir`, and the capture intake dir all anchor to the same project root.
## Design context (moved from source doc comments)

- `worker_thread_count(available)`: tokio worker-thread count = detected core count, but never below a hard floor of 4 (and 4 when detection fails). The floor's WHY is kept in source: the blocking bridges (tick distill, ingest embed, keepalive) each pin a worker; fewer workers starves the time driver and wedges the hub.
