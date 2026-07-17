# splinter: src/mcp.rs

Recovered from git (killed-agent gap):

- Module: the tool / prompt / resource handlers are built on the shared `tools::dispatch` core so the tool set has a single source of truth across the stdio and SSE/HTTP transports.
- `Server::cache`: shared, hence the `Mutex`. Holding a lock is acceptable because lookups/inserts are brief — a linear scan of a small bounded ring, not an unbounded index.
