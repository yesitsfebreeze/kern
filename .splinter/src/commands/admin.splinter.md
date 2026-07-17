# src/commands/admin.rs — commentary

- `cmd_health`: uses `graph_health_stats`, the same shared roll-up as `repl::do_health` and the MCP health tool, so the aggregation logic lives in exactly one place — don't reimplement counts here.
- `cmd_gc`: the runaway-bloat era saw `data.mdb` at ~4 GiB high-water mark; the daemon also reaps on startup, this command is the one-shot full clean (reap + file shrink) without the daemon.
